use std::io::{self, BufRead, Write};
use std::time::{Duration, Instant};

pub fn run(description: &str, duration_secs: u64) {
    let agent_id = std::env::var("CLAMOR_AGENT_ID").unwrap_or_else(|_| "unknown".into());

    println!("\x1b[1;36m══ Mock Agent ══\x1b[0m");
    println!("  id: {agent_id}");
    println!("  task: {description}");
    println!("  duration: {duration_secs}s");
    println!();

    // Spawn stdin echo thread
    std::thread::spawn(|| {
        let stdin = io::stdin();
        for line in stdin.lock().lines().map_while(Result::ok) {
            println!("\x1b[33mecho>\x1b[0m {line}");
            let _ = io::stdout().flush();
        }
    });

    // Periodic output
    let start = Instant::now();
    let duration = Duration::from_secs(duration_secs);
    let mut tick = 0u32;

    while start.elapsed() < duration {
        tick += 1;
        let elapsed = start.elapsed().as_secs();
        println!("[{elapsed:>3}s] tick {tick}");
        let _ = io::stdout().flush();
        std::thread::sleep(Duration::from_secs(2));
    }

    println!("\x1b[32mdone\x1b[0m");
}
