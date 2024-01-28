use core::panic;
use duct::cmd;
use signal_hook::{consts::SIGINT, iterator::Signals};
use std::{
    fs::{read_to_string, File},
    io::{stdout, BufRead, BufReader, Write},
    sync::mpsc::channel,
    thread::{self, sleep},
    time::{Duration, Instant},
};
use uuid::Uuid;

#[cfg(feature = "ubuntu")]
const PERF_HEADER_LINES: usize = 1;
#[cfg(feature = "ubuntu")]
const PERF_ENTRY_ADDITIONAL_LINES: usize = 0;

#[cfg(feature = "android")]
const PERF_HEADER_LINES: usize = 0;
#[cfg(feature = "android")]
const PERF_ENTRY_ADDITIONAL_LINES: usize = 2;

fn spawn_perf(interval: Duration) -> duct::ReaderHandle {
    if cfg!(feature = "ubuntu") {
        cmd!(
            "sh",
            "-c",
            format!(
                "sudo stdbuf -o0 -e0 perf stat -a --csv --interval-print {} -e cycles,context-switches --per-core",
                interval.as_millis()
            )
        )
        .stderr_to_stdout()
        .reader()
        .unwrap()
    } else if cfg!(feature = "android") {
        cmd!(
            "sh",
            "-c",
            format!(
                "sudo stdbuf -o0 -e0 simpleperf stat --use-devfreq-counters -a --csv --interval {} -e cpu-cycles,context-switches --per-core",
                interval.as_millis()
            )
        )
        .stderr_to_stdout()
        .reader()
        .unwrap()
    } else {
        panic!("Unknown System");
    }
}

fn process(
    output: &mut File,
    duration: Duration,
    interval: Duration,
    mut perf: duct::ReaderHandle,
) {
    let (perf_tx, perf_rx) = channel::<String>();
    thread::spawn(move || {
        let mut reader = BufReader::new(&mut perf);
        let mut line = String::new();

        for _ in 0..PERF_HEADER_LINES {
            reader.read_line(&mut line).unwrap();
        }
        line.clear();

        let cores = num_cpus::get_physical();

        let mut i = 0;
        loop {
            if let Ok(_) = reader.read_line(&mut line) {
                i += 1;
                if i % (cores * 2 + PERF_ENTRY_ADDITIONAL_LINES) == 0 {
                    perf_tx.send(line.clone()).unwrap();
                    line.clear();
                }
            } else {
                break;
            }
        }
    });

    let mut i = 0;
    let start_time = Instant::now();

    loop {
        let elapsed = start_time.elapsed();
        let proc_start = read_to_string("/proc/stat").unwrap();
        sleep(interval);
        let perf_entry = perf_rx.recv().unwrap();
        let proc_end = read_to_string("/proc/stat").unwrap();

        output
            .write(format!("<log-entry time=\"{}\">\n", elapsed.as_millis()).as_bytes())
            .unwrap();
        output
            .write(format!("<proc-start>\n{}</proc-start>\n", proc_start).as_bytes())
            .unwrap();
        output
            .write(format!("<proc-end>\n{}</proc-end>\n", proc_end).as_bytes())
            .unwrap();
        output
            .write(format!("<perf>\n{}</perf>\n", perf_entry).as_bytes())
            .unwrap();
        output.write(b"</log-entry>\n").unwrap();

        if elapsed >= duration {
            break;
        }

        i += 1;
        print!("\rLogged {} times", i);
        stdout().flush().unwrap();
    }
    println!();
}

fn main() {
    let mut args = std::env::args();
    let cmd = args.next().unwrap();
    if args.len() < 3 {
        println!("Usage: {} <output> <duration> <interval>", cmd);
        return;
    }
    let mut output = File::create(args.next().unwrap()).unwrap();

    let duration = Duration::from_secs(args.next().unwrap().parse::<u64>().unwrap());
    let interval = Duration::from_secs(args.next().unwrap().parse::<u64>().unwrap());
    let id = Uuid::new_v4();

    output
        .write(
            format!(
                "<log id=\"{}\" duration=\"{}\" interval=\"{}\">\n",
                id,
                duration.as_secs(),
                interval.as_secs()
            )
            .as_bytes(),
        )
        .unwrap();

    let perf = spawn_perf(interval);
    process(&mut output, duration, interval, perf);

    output.write(b"</log>\n").unwrap();
}
