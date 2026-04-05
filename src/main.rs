mod cli;
mod contract;
mod registry;
mod scaffold;

fn main() {
    if let Err(e) = cli::run() {
        eprintln!("{}: {}", console::style("error").red().bold(), e);
        std::process::exit(1);
    }
}
