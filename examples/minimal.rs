//! The least an app needs: its commands and what a line does.
//!
//!     cargo run -p shellkit --example minimal

use shellkit::{App, CommandInfo, Ctx, NoPanel, Shell, outln};

struct Hello;

impl App for Hello {
    type Msg = ();
    type Panel = NoPanel;

    fn commands(&self) -> Vec<CommandInfo> {
        vec![CommandInfo::new("greet", "Say hello").flag("--name", "Who to greet", Some("NAME"))]
    }

    fn run(&mut self, line: &str, ctx: &mut Ctx<Self>) {
        let words = shellkit::split_words(line);
        let name = words
            .iter()
            .position(|w| w == "--name")
            .and_then(|i| words.get(i + 1))
            .cloned()
            .unwrap_or_else(|| "there".into());
        ctx.start("Greeting", move || {
            std::thread::sleep(std::time::Duration::from_secs(1));
            outln!("Hello, {name}!");
        });
    }
}

fn main() -> anyhow::Result<()> {
    Shell::new(Hello).title("Hello").run()
}
