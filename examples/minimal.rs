//! The smallest program built on rak: one request, answer printed.
//! Run with: cargo run --example minimal -- "what does this project do?"

use rak::agent::{system_prompt, Agent, Config};
use rak::provider::OpenAICompatible;
use rak::tools::{core_tools, Ctx};

fn main() -> std::io::Result<()> {
    let task = std::env::args().nth(1).unwrap_or_else(|| "List the files here.".to_string());
    let cwd = std::env::current_dir()?;
    let ctx = Ctx::new(&cwd, false);
    let model = OpenAICompatible::new("http://localhost:11434/v1", None, "kimi-k2.6:cloud");
    let config = Config { system_prompt: system_prompt(&ctx.cwd, true, None), max_turns: 30, context_window: 128_000 };
    // Read-only tools: this example never changes anything.
    let mut agent = Agent::new(Box::new(model), core_tools(&ctx, true), config);
    let result = agent.run(&task);
    println!("{}", result.answer);
    Ok(())
}
