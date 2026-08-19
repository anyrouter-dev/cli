fn main() {
    let code = anyr_cli::run(
        std::env::args().skip(1).collect(),
        std::env::vars().collect(),
    );
    std::process::exit(code);
}
