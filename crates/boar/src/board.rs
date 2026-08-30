// Board daemon for concurrent BOAR allocation management
// Provides project-level quota management and fair allocation between concurrent builds

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::config::Settings;
use crate::memory::{MemInfo, Allocation, read_meminfo, calculate_allocation, human_kib};
use crate::filesystem::{project_target, directory_size};
use crate::estimate::{load_estimate, save_estimate};

type Result<T> = std::result::Result<T, String>;

const BOARD_LOCK_PATH: &str = "/tmp/boar-board.lock";
const BOARD_STATE_PATH: &str = "/tmp/boar-board-state.json";
const ALLOCATION_TIMEOUT_SECS: u64 = 300; // 5 minutes timeout
const QUOTA_DEFAULT_PERCENT: f64 = 0.8; // 80% of available RAM for quotas

#[derive(Clone, Debug)]
pub struct ProjectAllocation {
    pub project_path: String,
    pub ram_target: PathBuf,
    pub quota_mib: u64,
    pub current_usage_mib: u64,
    pub allocated_at: SystemTime,
    pub last_activity: SystemTime,
    pub build_count: u64,
}

#[derive(Clone, Debug)]
pub struct BoardState {
    pub total_ram_mib: u64,
    pub available_ram_mib: u64,
    pub quota_pool_mib: u64,
    pub projects: HashMap<String, ProjectAllocation>,
    pub active_allocations: u64,
    pub total_allocations: u64,
}

impl Default for BoardState {
    fn default() -> Self {
        Self {
            total_ram_mib: 0,
            available_ram_mib: 0,
            quota_pool_mib: 0,
            projects: HashMap::new(),
            active_allocations: 0,
            total_allocations: 0,
        }
    }
}

pub struct BoardDaemon {
    state: Arc<Mutex<BoardState>>,
    settings: Settings,
    running: Arc<Mutex<bool>>,
}

impl BoardDaemon {
    pub fn new(settings: Settings) -> Result<Self> {
        let state = Self::load_state().unwrap_or_default();
        Ok(Self {
            state: Arc::new(Mutex::new(state)),
            settings,
            running: Arc::new(Mutex::new(false)),
        })
    }

    fn load_state() -> Option<BoardState> {
        if let Ok(text) = fs::read_to_string(BOARD_STATE_PATH) {
            return Self::parse_state(&text);
        }
        None
    }

    fn parse_state(text: &str) -> Option<BoardState> {
        let mut state = BoardState::default();
        let mut in_projects = false;
        
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            
            if line == "projects:" {
                in_projects = true;
                continue;
            }
            
            if in_projects {
                if line.starts_with("  ") {
                    let parts: Vec<&str> = line[2..].split('|').collect();
                    if parts.len() >= 4 {
                        let quota = parts[1].parse().ok()?;
                        let usage = parts[2].parse().ok()?;
                        let builds = parts[3].parse().ok()?;
                        
                        let allocation = ProjectAllocation {
                            project_path: parts[0].to_string(),
                            ram_target: PathBuf::new(), // Will be reconstructed on demand
                            quota_mib: quota,
                            current_usage_mib: usage,
                            allocated_at: SystemTime::now(),
                            last_activity: SystemTime::now(),
                            build_count: builds,
                        };
                        state.projects.insert(parts[0].to_string(), allocation);
                    }
                }
            } else {
                if let Some((key, value)) = line.split_once('=') {
                    match key {
                        "total_ram_mib" => state.total_ram_mib = value.parse().ok()?,
                        "available_ram_mib" => state.available_ram_mib = value.parse().ok()?,
                        "quota_pool_mib" => state.quota_pool_mib = value.parse().ok()?,
                        "active_allocations" => state.active_allocations = value.parse().ok()?,
                        "total_allocations" => state.total_allocations = value.parse().ok()?,
                        _ => {}
                    }
                }
            }
        }
        
        Some(state)
    }

    fn save_state(&self) -> Result<()> {
        let state = self.state.lock().map_err(|e| format!("lock error: {e}"))?;
        let json = self.state_to_text(&*state)?;
        fs::write(BOARD_STATE_PATH, json)
            .map_err(|e| format!("write error: {e}"))?;
        Ok(())
    }

    fn state_to_text(&self, state: &BoardState) -> Result<String> {
        let mut text = String::new();
        text.push_str(&format!("total_ram_mib={}\n", state.total_ram_mib));
        text.push_str(&format!("available_ram_mib={}\n", state.available_ram_mib));
        text.push_str(&format!("quota_pool_mib={}\n", state.quota_pool_mib));
        text.push_str(&format!("active_allocations={}\n", state.active_allocations));
        text.push_str(&format!("total_allocations={}\n", state.total_allocations));
        text.push_str("projects:\n");
        
        for (key, alloc) in &state.projects {
            text.push_str(&format!("  {}|{}|{}|{}\n", 
                key, alloc.quota_mib, alloc.current_usage_mib, alloc.build_count));
        }
        
        Ok(text)
    }

    pub fn start(&self) -> Result<()> {
        {
            let mut running = self.running.lock().map_err(|e| format!("lock error: {e}"))?;
            if *running {
                return Err("Board daemon is already running".into());
            }
            *running = true;
        }

        // Check if another instance is running
        if Path::new(BOARD_LOCK_PATH).exists() {
            return Err("Board daemon lock file exists - another instance may be running".into());
        }

        // Create lock file
        fs::write(BOARD_LOCK_PATH, std::process::id().to_string())
            .map_err(|e| format!("cannot create lock file: {e}"))?;

        // Initialize state with current memory info
        self.update_memory_info()?;

        println!("BOAR Board daemon started");
        Ok(())
    }

    pub fn stop(&self) -> Result<()> {
        {
            let mut running = self.running.lock().map_err(|e| format!("lock error: {e}"))?;
            *running = false;
        }

        // Remove lock file
        if Path::new(BOARD_LOCK_PATH).exists() {
            fs::remove_file(BOARD_LOCK_PATH)
                .map_err(|e| format!("cannot remove lock file: {e}"))?;
        }

        // Save final state
        self.save_state()?;

        println!("BOAR Board daemon stopped");
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        self.running.lock()
            .map(|r| *r)
            .unwrap_or(false)
    }

    fn update_memory_info(&self) -> Result<()> {
        let memory = read_meminfo()?;
        let mut state = self.state.lock().map_err(|e| format!("lock error: {e}"))?;
        
        state.total_ram_mib = memory.total_kib / 1024;
        state.available_ram_mib = memory.available_kib / 1024;
        state.quota_pool_mib = (state.available_ram_mib as f64 * QUOTA_DEFAULT_PERCENT) as u64;
        
        Ok(())
    }

    pub fn request_allocation(&self, project_path: &Path, estimated_mib: u64) -> Result<Allocation> {
        self.update_memory_info()?;
        
        let project_key = project_path.to_string_lossy().to_string();
        let memory = read_meminfo()?;
        
        let mut state = self.state.lock().map_err(|e| format!("lock error: {e}"))?;
        
        // Check if project already has allocation
        if let Some(existing) = state.projects.get(&project_key) {
            // Update last activity
            let mut updated = existing.clone();
            updated.last_activity = SystemTime::now();
            updated.build_count += 1;
            state.projects.insert(project_key.clone(), updated);
            
            // Return existing allocation
            let allocation = calculate_allocation(
                memory,
                existing.quota_mib * 1024, // Convert back to KiB
                Some(existing.quota_mib),
                None,
                estimated_mib * 1024,
            );
            
            self.save_state()?;
            return Ok(allocation);
        }

        // Calculate fair quota based on available pool and active projects
        let active_count = state.projects.len() as u64;
        let fair_quota = if active_count > 0 {
            state.quota_pool_mib / (active_count + 1)
        } else {
            state.quota_pool_mib
        };

        // Ensure minimum quota
        let quota = fair_quota.max(512); // Minimum 512 MiB per project

        // Create new project allocation
        let ram_target = project_target(&self.settings.ram_root, project_path);
        let allocation = ProjectAllocation {
            project_path: project_key.clone(),
            ram_target: ram_target.clone(),
            quota_mib: quota,
            current_usage_mib: 0,
            allocated_at: SystemTime::now(),
            last_activity: SystemTime::now(),
            build_count: 1,
        };

        state.projects.insert(project_key, allocation);
        state.active_allocations += 1;
        state.total_allocations += 1;

        // Calculate allocation based on quota
        let result_allocation = calculate_allocation(
            memory,
            quota * 1024, // Convert to KiB
            Some(quota),
            None,
            estimated_mib * 1024,
        );

        self.save_state()?;
        Ok(result_allocation)
    }

    pub fn release_allocation(&self, project_path: &Path) -> Result<()> {
        let project_key = project_path.to_string_lossy().to_string();
        let mut state = self.state.lock().map_err(|e| format!("lock error: {e}"))?;
        
        if let Some(allocation) = state.projects.remove(&project_key) {
            state.active_allocations -= 1;
            
            // Update current usage before release
            if let Ok(size_kib) = directory_size(&allocation.ram_target) {
                // Could log final usage statistics here
            }
        }

        self.save_state()?;
        Ok(())
    }

    pub fn update_usage(&self, project_path: &Path, current_mib: u64) -> Result<()> {
        let project_key = project_path.to_string_lossy().to_string();
        let mut state = self.state.lock().map_err(|e| format!("lock error: {e}"))?;
        
        if let Some(allocation) = state.projects.get_mut(&project_key) {
            allocation.current_usage_mib = current_mib;
            allocation.last_activity = SystemTime::now();
        }

        self.save_state()?;
        Ok(())
    }

    pub fn get_state(&self) -> Result<BoardState> {
        let state = self.state.lock().map_err(|e| format!("lock error: {e}"))?;
        Ok(state.clone())
    }

    pub fn get_project_allocation(&self, project_path: &Path) -> Option<ProjectAllocation> {
        let project_key = project_path.to_string_lossy().to_string();
        let state = self.state.lock().ok()?;
        state.projects.get(&project_key).cloned()
    }

    pub fn cleanup_stale_allocations(&self) -> Result<u64> {
        let now = SystemTime::now();
        let timeout = Duration::from_secs(ALLOCATION_TIMEOUT_SECS);
        let mut cleaned = 0;
        
        let mut state = self.state.lock().map_err(|e| format!("lock error: {e}"))?;
        
        let mut stale_projects: Vec<String> = Vec::new();
        for (key, allocation) in &state.projects {
            if let Ok(elapsed) = now.duration_since(allocation.last_activity) {
                if elapsed > timeout {
                    stale_projects.push(key.clone());
                }
            }
        }

        for key in stale_projects {
            if state.projects.remove(&key).is_some() {
                state.active_allocations -= 1;
                cleaned += 1;
            }
        }

        if cleaned > 0 {
            self.save_state()?;
        }

        Ok(cleaned)
    }

    pub fn enforce_quotas(&self) -> Result<Vec<String>> {
        let mut violations = Vec::new();
        let mut state = self.state.lock().map_err(|e| format!("lock error: {e}"))?;
        
        for (key, allocation) in &state.projects {
            if allocation.current_usage_mib > allocation.quota_mib {
                violations.push(format!(
                    "Project {} exceeds quota: {} MiB used / {} MiB quota",
                    key, allocation.current_usage_mib, allocation.quota_mib
                ));
            }
        }

        Ok(violations)
    }

    pub fn print_status(&self) -> Result<()> {
        let state = self.get_state()?;
        
        println!("=== BOAR Board Daemon Status ===");
        println!("Total RAM: {} MiB", state.total_ram_mib);
        println!("Available RAM: {} MiB", state.available_ram_mib);
        println!("Quota Pool: {} MiB", state.quota_pool_mib);
        println!("Active Allocations: {}", state.active_allocations);
        println!("Total Allocations: {}", state.total_allocations);
        println!();
        
        if state.projects.is_empty() {
            println!("No active project allocations.");
        } else {
            println!("Active Projects:");
            for (key, allocation) in &state.projects {
                let elapsed = allocation.last_activity
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                println!("  {}: {} MiB quota, {} MiB used, {} builds, last active {}s ago",
                    key,
                    allocation.quota_mib,
                    allocation.current_usage_mib,
                    allocation.build_count,
                    elapsed
                );
            }
        }
        
        Ok(())
    }
}

impl Drop for BoardDaemon {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

// Simple JSON serialization for BoardState (since we're zero-dependency)
// In a real implementation, we'd use serde, but for now we'll use a simple approach
mod simple_json {
    use super::*;
    
    pub fn to_string(state: &BoardState) -> Result<String> {
        // Simple JSON-like format for state persistence
        let mut json = String::new();
        json.push_str("{\n");
        json.push_str(&format!("  \"total_ram_mib\": {},\n", state.total_ram_mib));
        json.push_str(&format!("  \"available_ram_mib\": {},\n", state.available_ram_mib));
        json.push_str(&format!("  \"quota_pool_mib\": {},\n", state.quota_pool_mib));
        json.push_str(&format!("  \"active_allocations\": {},\n", state.active_allocations));
        json.push_str(&format!("  \"total_allocations\": {},\n", state.total_allocations));
        json.push_str("  \"projects\": {\n");
        
        for (i, (key, alloc)) in state.projects.iter().enumerate() {
            if i > 0 {
                json.push_str(",\n");
            }
            json.push_str(&format!("    \"{}\": {{\n", key));
            json.push_str(&format!("      \"quota_mib\": {},\n", alloc.quota_mib));
            json.push_str(&format!("      \"current_usage_mib\": {},\n", alloc.current_usage_mib));
            json.push_str(&format!("      \"build_count\": {}\n", alloc.build_count));
            json.push_str("    }");
        }
        
        json.push_str("\n  }\n}\n");
        Ok(json)
    }
    
    pub fn from_str(_json: &str) -> Result<BoardState> {
        // For simplicity, return default state
        // In production, this would parse the JSON properly
        Ok(BoardState::default())
    }
}

// Use simple JSON as serde substitute for zero-dependency requirement
pub fn serde_json_to_string(state: &BoardState) -> Result<String> {
    simple_json::to_string(state)
}

pub fn serde_json_from_str(json: &str) -> Result<BoardState> {
    simple_json::from_str(json)
}

// Re-export for compatibility
pub use serde_json_to_string as serde_json_to_string_pretty;
pub use serde_json_from_str as serde_json_from_str;