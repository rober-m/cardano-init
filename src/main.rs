mod cli;
mod contract;
mod registry;
mod scaffold;
mod web;

fn main() {
    if let Err(e) = cli::run() {
        eprintln!("{}: {}", console::style("error").red().bold(), e);
        std::process::exit(1);
    }
}
