//! The command line: the options and the help text.

pub use super::VERSION;
use std::path::PathBuf;

pub const HELP: &str = "\
rak — Rust Agent Kit: a small coding agent for the terminal

USAGE
  rak [options] \"what to do\"      one request, then exit
  rak [options]                   chat (type / for the commands, /exit to leave)
  echo \"what to do\" | rak         the request from a pipe
  rak keys                        set up ~/.config/rak/.env and see which keys are set
  rak keys --update               bring an older key file up to date, keeping its values
  rak mcp add|list|remove         tools from MCP servers (see `rak mcp --help`)

MODEL
  -m, --model NAME        model to use                      [env RAK_MODEL]
      --provider NAME     who runs the model; each gets its own address and key
                          ollama, ollama-cloud, anthropic, openai, openrouter,
                          huggingface, groq, deepseek, mistral, gemini, zai,
                          zai-coding, lmstudio, llamacpp, vllm   [env RAK_PROVIDER]
                          default: ollama, or anthropic for a claude-* model
                          when ANTHROPIC_API_KEY is set
      --base-url URL      another address: a server rak doesn't know by name,
                          or a provider somewhere else            [env RAK_BASE_URL]
      --api-key-env NAME  the variable holding the key to send
                          default: the provider's own key (`rak keys` lists
                          them); RAK_API_KEY for a server named by --base-url
      --effort LEVEL      reasoning effort: low, medium, high, xhigh or max
      --max-tokens N      longest reply to ask for
      --context N         the model's context window in tokens
                          default: what an Ollama server reports, else 128000
      --no-stream         wait for each whole reply instead of showing it as it arrives

  In a chat, the model can be left out: rak uses the one picked last with
  /model, or lists the server's models to pick from.

KEYS (run `rak keys` to create the key file from a template)
  Read from the environment, then ./.env in the project, then
  ~/.config/rak/.env. Values from the files are not passed on to commands.
  Each provider has its own key (ANTHROPIC_API_KEY, OPENROUTER_API_KEY,
  OLLAMA_API_KEY, …); `rak keys` lists them all. RAK_PROVIDER, RAK_MODEL and
  RAK_BASE_URL may be set there too.
  TAVILY_API_KEY                                   web_search (https://tavily.com)

BEHAVIOUR
  -y, --yes               run every tool without asking
      --read-only         no edits and no commands: read and report
      --any-path          let file tools reach outside the project folder
      --no-mcp            don't connect the MCP servers for this run
      --no-skills         no skills for this run, built-in ones included
  -c, --continue          continue this folder's last chat (chats are saved
                          after every request; /resume lists them)
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

#[derive(Clone)]
pub struct Options {
    pub task: Option<String>,
    pub model: Option<String>,
    /// "openai" or "anthropic"; None: decided from the model and address.
    pub provider: Option<String>,
    /// None: the provider's default address.
    pub base_url: Option<String>,
    /// --provider / --base-url were on the command line, not defaults from
    /// RAK_PROVIDER / RAK_BASE_URL: a flag never mixes with the other's
    /// default, so one provider's key never reaches another's server.
    pub provider_flag: bool,
    pub base_url_flag: bool,
    /// -m was on the command line: it wins over the last pick and RAK_MODEL.
    pub model_flag: bool,
    pub api_key_env: Option<String>,
    pub effort: Option<String>,
    pub max_tokens: Option<u64>,
    pub context: Option<usize>,
    pub no_stream: bool,
    pub yes: bool,
    pub read_only: bool,
    pub any_path: bool,
    pub no_mcp: bool,
    pub no_skills: bool,
    pub continue_last: bool,
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
        provider_flag: false,
        base_url_flag: false,
        model_flag: false,
        api_key_env: None,
        effort: None,
        max_tokens: None,
        context: None,
        no_stream: false,
        yes: false,
        read_only: false,
        any_path: false,
        no_mcp: false,
        no_skills: false,
        continue_last: false,
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
            "-m" | "--model" => {
                o.model = Some(value("--model"));
                o.model_flag = true;
            }
            "--base-url" => {
                o.base_url = Some(value("--base-url"));
                o.base_url_flag = true;
            }
            "--provider" => {
                o.provider = Some(value("--provider"));
                o.provider_flag = true;
            }
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
            "--no-skills" => o.no_skills = true,
            "-c" | "--continue" => o.continue_last = true,
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
    if let Some(provider) = o.provider.as_deref().filter(|p| rak::providers::find(p).is_none()) {
        usage_error(&format!("unknown provider `{provider}`. Known: {}", rak::providers::names()));
    }
    o
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_help_names_every_provider() {
        let normalized = super::HELP.replace(['\n', ' '], ",");
        for provider in rak::providers::all() {
            assert!(normalized.split(',').any(|word| word == provider.name), "--help does not name {}", provider.name);
        }
    }
}
