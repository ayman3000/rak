//! The command line: the options and the help text.

pub use super::VERSION;
use std::path::PathBuf;

pub const HELP: &str = "\
rak — Rust Agent Kit: a small coding agent for the terminal

USAGE
  rak [options] \"what to do\"      one request, then exit
  rak [options]                   chat (type / for the commands, /exit to leave)
  echo \"what to do\" | rak         the request from a pipe
  rak mcp add|list|remove         tools from MCP servers (see `rak mcp --help`)

MODEL
  -m, --model NAME        model to use                      [env RAK_MODEL]
      --provider NAME     openai: any server that speaks OpenAI's chat-completions
                          API (Ollama, OpenAI, OpenRouter, vLLM…)
                          anthropic: Claude through Anthropic's own API, with
                          prompt caching                    [env RAK_PROVIDER]
                          default: anthropic for a claude-* model when
                          ANTHROPIC_API_KEY is set and no --base-url is given,
                          or when the address is Anthropic's; otherwise openai
      --base-url URL      server address                    [env RAK_BASE_URL]
                          default: http://localhost:11434/v1 (Ollama), or
                          https://api.anthropic.com for anthropic
      --api-key-env NAME  environment variable holding the key
                          default: RAK_API_KEY, then OPENAI_API_KEY;
                          ANTHROPIC_API_KEY first for anthropic
      --effort LEVEL      reasoning effort: low, medium, high, xhigh or max
      --max-tokens N      longest reply to ask for
      --context N         the model's context window in tokens
                          default: what an Ollama server reports, else 128000
      --no-stream         wait for each whole reply instead of showing it as it arrives

  In a chat, the model can be left out: rak uses the one picked last with
  /model, or lists the server's models to pick from.

KEYS
  Read from the environment, then ./.env in the project, then
  ~/.config/rak/.env. Values from the files are not passed on to commands.
  ANTHROPIC_API_KEY, RAK_API_KEY, OPENAI_API_KEY   the model server
  TAVILY_API_KEY                                   web_search (https://tavily.com)

BEHAVIOUR
  -y, --yes               run every tool without asking
      --read-only         no edits and no commands: read and report
      --any-path          let file tools reach outside the project folder
      --no-mcp            don't connect the MCP servers for this run
  -C, --cwd DIR           the project folder (default: where rak was started)
      --max-turns N       model calls allowed per request (default 30)

OUTPUT
      --json              print one JSON object with the answer and the numbers
      --log FILE          append every step to FILE as JSON lines
  -q, --quiet             no progress lines, only the answer
  -h, --help              this text
  -V, --version           the version

EXIT CODES
  0 done   1 failed   2 bad usage   3 a tool was not approved   4 turn limit   5 stuck repeating a call
  130 stopped with Ctrl-C
";

pub struct Options {
    pub task: Option<String>,
    pub model: Option<String>,
    /// "openai" or "anthropic"; None: decided from the model and address.
    pub provider: Option<String>,
    /// None: the provider's default address.
    pub base_url: Option<String>,
    pub api_key_env: Option<String>,
    pub effort: Option<String>,
    pub max_tokens: Option<u64>,
    pub context: Option<usize>,
    pub no_stream: bool,
    pub yes: bool,
    pub read_only: bool,
    pub any_path: bool,
    pub no_mcp: bool,
    pub cwd: Option<PathBuf>,
    pub max_turns: usize,
    pub json: bool,
    pub log: Option<PathBuf>,
    pub quiet: bool,
}

pub fn usage_error(message: &str) -> ! {
    eprintln!("rak: {message}\nTry `rak --help`.");
    std::process::exit(2);
}

pub fn parse_args(args: Vec<String>) -> Options {
    let env = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let mut o = Options {
        task: None,
        model: env("RAK_MODEL"),
        provider: env("RAK_PROVIDER"),
        base_url: env("RAK_BASE_URL"),
        api_key_env: None,
        effort: None,
        max_tokens: None,
        context: None,
        no_stream: false,
        yes: false,
        read_only: false,
        any_path: false,
        no_mcp: false,
        cwd: None,
        max_turns: 30,
        json: false,
        log: None,
        quiet: false,
    };
    let mut words: Vec<String> = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().unwrap_or_else(|| usage_error(&format!("{name} needs a value")));
        fn number<T: std::str::FromStr>(name: &str, text: String) -> T {
            text.parse().unwrap_or_else(|_| usage_error(&format!("{name} needs a number, not `{text}`")))
        }
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("rak {VERSION}");
                std::process::exit(0);
            }
            "-m" | "--model" => o.model = Some(value("--model")),
            "--base-url" => o.base_url = Some(value("--base-url")),
            "--provider" => o.provider = Some(value("--provider")),
            "--api-key-env" => o.api_key_env = Some(value("--api-key-env")),
            "--effort" => {
                let level = value("--effort");
                if !["low", "medium", "high", "xhigh", "max"].contains(&level.as_str()) {
                    usage_error("--effort is low, medium, high, xhigh or max");
                }
                o.effort = Some(level);
            }
            "--max-tokens" => o.max_tokens = Some(number("--max-tokens", value("--max-tokens"))),
            "--context" => o.context = Some(number("--context", value("--context"))),
            "--no-stream" => o.no_stream = true,
            "-y" | "--yes" => o.yes = true,
            "--read-only" => o.read_only = true,
            "--any-path" => o.any_path = true,
            "--no-mcp" => o.no_mcp = true,
            "-C" | "--cwd" => o.cwd = Some(PathBuf::from(value("--cwd"))),
            "--max-turns" => o.max_turns = number::<usize>("--max-turns", value("--max-turns")).max(1),
            "--json" => o.json = true,
            "--log" => o.log = Some(PathBuf::from(value("--log"))),
            "-q" | "--quiet" => o.quiet = true,
            "--" => words.extend(args.by_ref()),
            flag if flag.starts_with('-') && flag.len() > 1 => usage_error(&format!("unknown option {flag}")),
            _ => words.push(arg),
        }
    }
    if !words.is_empty() {
        o.task = Some(words.join(" "));
    }
    if let Some(provider) = &o.provider {
        if provider != "openai" && provider != "anthropic" {
            usage_error("--provider is openai or anthropic");
        }
    }
    o
}
