use chrono::Duration;
use dhan_token::TokenInfo;
use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let token = match env::var("DHAN_ACCESS_TOKEN") {
        Ok(t) if !t.trim().is_empty() => t,
        _ => {
            eprintln!("Error: DHAN_ACCESS_TOKEN environment variable is not set or empty.");
            return ExitCode::from(1);
        }
    };

    let info = match TokenInfo::parse(&token) {
        Ok(info) => info,
        Err(e) => {
            eprintln!("Error parsing token: {e} (code: {})", e.code());
            return ExitCode::from(1);
        }
    };

    println!("Dhan Client ID: {}", info.client_id);
    println!("Expiry (IST):   {}", info.expires_at_ist_string());
    println!("Expiry (UTC):   {}", info.expires_at().format("%Y-%m-%d %H:%M:%S UTC"));

    let margin = Duration::minutes(2);
    let status = info.status_now(margin);
    println!("Status (2m margin): {status}");

    let remaining = info.remaining_now();
    println!("Remaining:      {} seconds", remaining.num_seconds());

    if info.is_expired_now(Duration::zero()) {
        eprintln!("Token is expired!");
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}
