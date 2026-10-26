use anyhow::{Context, Result, ensure};
use std::{env, fs};

fn main() -> Result<()> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    ensure!(args.len() == 2, "Expected artifact and signature paths");
    let bytes = fs::read(&args[0]).context("Read release artifact")?;
    let signature = fs::read_to_string(&args[1]).context("Read release signature")?;
    ratatoskr_core::updates::verify(
        include_str!("../src/update-public-key.txt"),
        &bytes,
        &signature,
    )?;
    println!("Verified release artifact against the existing update trust root");
    Ok(())
}
