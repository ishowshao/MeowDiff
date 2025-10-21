use meowdiff::run_cli;

#[tokio::main]
async fn main() {
    if let Err(err) = run_cli().await {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}
