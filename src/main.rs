//! The `rak` command: one request from the command line, or a chat when
//! started without one. Everything it does on the terminal is in src/cli;
//! the agent itself is the library (src/lib.rs).

mod cli;

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.first().map(String::as_str) == Some("mcp") {
        cli::mcp_command::run(&argv[1..]);
    }
    if argv.first().map(String::as_str) == Some("keys") {
        cli::keys_command::run(&argv[1..]);
    }
    let options = cli::args::parse_args(argv);
    std::process::exit(cli::session::run(options));
}
