use anyhow::Result;

use crate::cli::OrchestrateGrammarArgs;
use crate::orchestrate::grammar as orch_grammar;

pub fn grammar(args: OrchestrateGrammarArgs) -> Result<()> {
    let OrchestrateGrammarArgs { markdown, json } = args;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&orch_grammar::render_json())?
        );
    } else if markdown {
        print!("{}", orch_grammar::render_markdown());
    } else {
        orch_grammar::render_human();
    }
    Ok(())
}
