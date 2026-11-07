mod config;

use config::Config;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load()?;
    println!("Listening on: {}", config.dns.listen);
    println!("Upstreams: {:?}", config.dns.upstreams);
    println!("Rewrites: {:?}", config.rewrites);
    Ok(())
}
