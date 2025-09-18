//! A simple but realistic load‑tester for the Fractal RPC.
//! It mixes the various endpoints (program accounts, token accounts, simulateTransaction)
//! and reports latency percentiles.

use {
    clap::Parser,
    futures::future::join_all,
    rand::seq::SliceRandom,
    reqwest::Client,
    serde_json::json,
    std::time::Instant,
    tokio::time::{sleep, Duration},
};

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Target RPC endpoint (e.g. http://127.0.0.1:8899)
    #[arg(long, default_value = "http://127.0.0.1:8899")]
    endpoint: String,

    /// How long to run the benchmark
    #[arg(long, default_value = "30s")]
    duration: humantime::Duration,

    /// Number of concurrent tasks
    #[arg(long, default_value = "12")]
    concurrency: usize,

    /// Number of HTTP connections to keep alive
    #[arg(long, default_value = "400")]
    connections: usize,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let client = Client::builder()
        .pool_max_idle_per_host(args.connections)
        .build()
        .unwrap();

    // A small pool of example program IDs and token mints.
    let programs = vec![
        "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", // SPL Token
        "11111111111111111111111111111111", // System program (should be fast)
    ];
    let token_mints = vec![
        "So11111111111111111111111111111111111111112", // Wrapped SOL
    ];

    let end_time = Instant::now() + args.duration.into();

    // Collect latency samples.
    let mut latencies: Vec<f64> = Vec::new();

    // Spawn the requested number of concurrent workers.
    let mut workers = Vec::new();
    for _ in 0..args.concurrency {
        let client = client.clone();
        let endpoint = args.endpoint.clone();
        let programs = programs.clone();
        let token_mints = token_mints.clone();

        workers.push(tokio::spawn(async move {
            let mut rng = rand::thread_rng();
            while Instant::now() < end_time {
                // Randomly pick an endpoint to hit.
                let choice = rand::random::<u8>() % 4;
                let start = Instant::now();
                match choice {
                    0 => {
                        // getProgramAccounts (no pagination)
                        let prog = programs.choose(&mut rng).unwrap();
                        let body = json!({ "program": prog });
                        let _ = client
                            .post(&format!("{}/getProgramAccounts", endpoint))
                            .json(&body)
                            .send()
                            .await;
                    }
                    1 => {
                        // getTokenAccountsByOwner (fast path)
                        let owner = programs.choose(&mut rng).unwrap(); // use program as fake owner
                        let body = json!({ "owner": owner });
                        let _ = client
                            .post(&format!("{}/getTokenAccountsByOwner", endpoint))
                            .json(&body)
                            .send()
                            .await;
                    }
                    2 => {
                        // simulateTransaction (proxy)
                        let fake_tx = json!({
                            "jsonrpc": "2.0",
                            "id": 1,
                            "method": "simulateTransaction",
                            "params": {
                                "transaction": "AQAB...", // truncated dummy base64 tx
                                "sigVerify": false,
                                "replaceRecentBlockhash": true,
                                "encoding": "base64"
                            }
                        });
                        let _ = client
                            .post(&format!("{}/simulateTransaction", endpoint))
                            .json(&fake_tx)
                            .send()
                            .await;
                    }
                    _ => {
                        // getMultipleAccounts (2 random pubkeys)
                        let pk1 = programs.choose(&mut rng).unwrap();
                        let pk2 = token_mints.choose(&mut rng).unwrap();
                        let body = json!({ "pubkeys": [pk1, pk2] });
                        let _ = client
                            .post(&format!("{}/getMultipleAccounts", endpoint))
                            .json(&body)
                            .send()
                            .await;
                    }
                }
                let elapsed = start.elapsed().as_secs_f64() * 1000.0; // ms
                latencies.push(elapsed);
                // Small back‑off to avoid hammering the same endpoint too fast.
                sleep(Duration::from_millis(5)).await;
            }
        }));
    }

    // Wait for all workers to finish.
    join_all(workers).await;

    // Compute percentiles.
    latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p50 = latencies[(latencies.len() as f64 * 0.50) as usize];
    let p95 = latencies[(latencies.len() as f64 * 0.95) as usize];
    let p99 = latencies[(latencies.len() as f64 * 0.99) as usize];
    let max = *latencies.last().unwrap_or(&0.0);
    let avg: f64 = latencies.iter().sum::<f64>() / latencies.len() as f64;

    println!("=== Benchmark results ===");
    println!("Requests sent: {}", latencies.len());
    println!("Avg latency:   {:.2} ms", avg);
    println!("p50 latency:   {:.2} ms", p50);
    println!("p95 latency:   {:.2} ms", p95);
    println!("p99 latency:   {:.2} ms", p99);
    println!("Max latency:   {:.2} ms", max);
                  }
                      
