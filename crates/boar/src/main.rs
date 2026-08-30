fn main() {
    let code = match boar::run(std::env::args().skip(1).collect()) {
        Ok(code) => code,
        Err(message) => {
            let support = boar::style::stderr_support();
            eprintln!("{} {message}", boar::style::error("error:", support));
            2
        }
    };

    std::process::exit(code);
}
