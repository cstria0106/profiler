use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use lazy_regex::regex_captures;
use quick_xml::Reader;
use regex::Regex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
struct LogEntry {
    time: String,
    #[serde(rename = "proc-start")]
    proc_start: String,
    #[serde(rename = "proc-end")]
    proc_end: String,
    perf: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Log {
    id: String,
    duration: String,
    interval: String,
    #[serde(rename = "$value")]
    entries: Vec<LogEntry>,
}

#[derive(Debug)]
struct ProcReport {
    user: u64,
    nice: u64,
    system: u64,
    idle: u64,
    iowait: u64,
    irq: u64,
    softirq: u64,
    total: u64,
    load: f64,
}

struct ProcReportPoint {
    user: u64,
    nice: u64,
    system: u64,
    idle: u64,
    iowait: u64,
    irq: u64,
    softirq: u64,
    total: u64,
}

#[derive(Debug)]
struct PerfReport {
    cycles: u64,
    context_switches: u64,
}

#[derive(Debug)]
struct ReportEntry {
    time: Duration,
    proc: HashMap<String, ProcReport>,
    perf: HashMap<String, PerfReport>,
}

#[derive(Debug)]
struct Report {
    id: Uuid,
    duration: Duration,
    interval: Duration,
    entries: Vec<ReportEntry>,
    proc_cpus: HashSet<String>,
    perf_cpus: HashSet<String>,
}

fn parse_proc(text: &str) -> HashMap<String, ProcReportPoint> {
    let mut proc: HashMap<String, ProcReportPoint> = HashMap::new();
    let lines = text.lines();

    for line in lines {
        let matches = regex_captures!(
            r#"cpu(\d+| ) (\d+) (\d+) (\d+) (\d+) (\d+) (\d+) (\d+) \d+ \d+ \d+"#,
            line
        );
        if let Some((_, cpu, user, system, nice, idle, iowait, irq, softirq)) = matches {
            let (user, system, nice, idle, iowait, irq, softirq) = (
                user.parse().unwrap(),
                system.parse().unwrap(),
                nice.parse().unwrap(),
                idle.parse().unwrap(),
                iowait.parse().unwrap(),
                irq.parse().unwrap(),
                softirq.parse().unwrap(),
            );
            let total = user + system + nice + idle + iowait + irq + softirq;
            let proc_report = ProcReportPoint {
                user,
                nice,
                system,
                idle,
                iowait,
                irq,
                softirq,
                total,
            };

            proc.insert(cpu.to_owned(), proc_report);
        }
    }

    proc
}

fn analyze_proc(start: &ProcReportPoint, end: &ProcReportPoint) -> ProcReport {
    ProcReport {
        user: end.user - start.user,
        nice: end.nice - start.nice,
        system: end.system - start.system,
        idle: end.idle - start.idle,
        iowait: end.iowait - start.iowait,
        irq: end.irq - start.irq,
        softirq: end.softirq - start.softirq,
        total: end.total - start.total,
        load: 100.0 * (1.0 - (end.idle - start.idle) as f64 / (end.total - start.total) as f64),
    }
}

fn parse_and_analyze_perf(text: &str) -> HashMap<String, PerfReport> {
    let mut perf: HashMap<String, PerfReport> = HashMap::new();
    let lines = text.lines();
    for line in lines {
        let matches = if cfg!(feature = "android") {
            regex_captures!(r#"(\d)+,(\d+),(.+?),.+?,.+?,.*"#, line)
        } else if cfg!(feature = "ubuntu") {
            regex_captures!(r#"\d+\.\d+\s+(.+?\-.+?\-.+?)\s+\d+\s+(\d+)\s+(.+)"#, line)
        } else {
            panic!("Unknown System");
        };

        if let Some((_, cpu, value, event)) = matches {
            let value = value.parse().unwrap();

            let perf = perf.entry(cpu.to_string()).or_insert(PerfReport {
                cycles: 0,
                context_switches: 0,
            });

            if event
                == if cfg!(feature = "android") {
                    "cpu-cycles"
                } else if cfg!(feature = "ubuntu") {
                    "cycles"
                } else {
                    panic!("Unknown system");
                }
            {
                perf.cycles = value;
            } else if event == "context-switches" {
                perf.context_switches = value;
            }
        }
    }

    perf
}

fn main() {
    let mut args = std::env::args();
    let cmd = args.next().unwrap();
    if args.len() != 1 {
        println!("Usage: {} <input>", cmd);
    }

    let xml = std::fs::read_to_string(args.next().unwrap()).unwrap();

    let log = serde_xml_rs::from_str::<Log>(&xml).unwrap();

    let mut report_entries: Vec<ReportEntry> = Vec::new();
    let mut proc_cpus = HashSet::new();
    let mut perf_cpus = HashSet::new();

    for log_entry in log.entries {
        let proc_start = parse_proc(&log_entry.proc_start);
        let proc_end = parse_proc(&log_entry.proc_end);
        let proc = HashMap::from_iter(proc_start.keys().map(|cpu| {
            if !proc_end.contains_key(cpu) {
                panic!("CPU {} not found in proc_end", cpu);
            }

            (cpu.clone(), analyze_proc(&proc_start[cpu], &proc_end[cpu]))
        }));

        let perf = parse_and_analyze_perf(&log_entry.perf);

        for cpu in proc.keys() {
            proc_cpus.insert(cpu.clone());
        }

        for cpu in perf.keys() {
            perf_cpus.insert(cpu.clone());
        }

        report_entries.push(ReportEntry {
            time: Duration::from_secs(log_entry.time.parse().unwrap()),
            proc,
            perf,
        });
    }

    let report = Report {
        id: log.id.parse().unwrap(),
        duration: Duration::from_secs(log.duration.parse().unwrap()),
        interval: Duration::from_secs(log.interval.parse().unwrap()),
        entries: report_entries,
        perf_cpus,
        proc_cpus,
    };

    println!("{:#?}", report);
}
