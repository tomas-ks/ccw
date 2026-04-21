fn main() {
    match cc_w_platform_headless::run_from_env() {
        Ok(message) => println!("{message}"),
        Err(error) => {
            eprintln!("{error}");
            if error.is_usage() {
                eprintln!();
                eprintln!("{}", cc_w_platform_headless::usage());
            }
            std::process::exit(2);
        }
    }
}
