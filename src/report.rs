use std::{
    collections::{HashMap, HashSet},
    process::exit,
    time::Duration,
};

use colored::Colorize;
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
    proc_cpus: Vec<String>,
    perf_cpus: Vec<String>,
}

fn parse_proc(text: &str) -> HashMap<String, ProcReportPoint> {
    let mut proc: HashMap<String, ProcReportPoint> = HashMap::new();
    let lines = text.lines();

    for line in lines {
        let matches = regex_captures!(
            r#"cpu(\d+| ) (\d+) (\d+) (\d+) (\d+) (\d+) (\d+) (\d+) \d+ \d+ \d+"#,
            line
        );
        if let Some((_, mut cpu, user, system, nice, idle, iowait, irq, softirq)) = matches {
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

            if cpu == " " {
                cpu = "all";
            }

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
            let event = event.trim();

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
            } else {
                panic!("Unknown event: \"{}\"", event);
            }
        }
    }

    for cpu in perf.keys().cloned().collect::<Vec<_>>() {
        let cycles = perf[&cpu].cycles;
        let context_switches = perf[&cpu].context_switches;

        let all = perf.entry("all".to_string()).or_insert(PerfReport {
            cycles: 0,
            context_switches: 0,
        });

        all.cycles += cycles;
        all.context_switches += context_switches;
    }

    perf
}

fn get_report(xml: String) -> Report {
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

    let mut perf_cpus: Vec<_> = perf_cpus.into_iter().collect();
    let mut proc_cpus: Vec<_> = proc_cpus.into_iter().collect();
    let compare = |a: &String, b: &String| {
        if a == "all" {
            std::cmp::Ordering::Greater
        } else if b == "all" {
            std::cmp::Ordering::Less
        } else if a.len() < b.len() {
            std::cmp::Ordering::Less
        } else if a.len() > b.len() {
            std::cmp::Ordering::Greater
        } else {
            a.cmp(b)
        }
    };
    perf_cpus.sort_by(compare);
    proc_cpus.sort_by(compare);

    Report {
        id: log.id.parse().unwrap(),
        duration: Duration::from_secs(log.duration.parse().unwrap()),
        interval: Duration::from_secs(log.interval.parse().unwrap()),
        entries: report_entries,
        perf_cpus,
        proc_cpus,
    }
}

fn get_average_proc_load(report: &Report, cpu: &str) -> f64 {
    let mut total = 0.0;
    let mut count = 0;
    for entry in report.entries.iter() {
        if entry.proc.contains_key(cpu) {
            total += entry.proc[cpu].load;
            count += 1;
        }
    }

    total / count as f64
}

fn get_average_cpu_cycles(report: &Report, cpu: &str) -> f64 {
    let mut total = 0.0;
    let mut count = 0;
    for entry in report.entries.iter() {
        if entry.perf.contains_key(cpu) {
            total += entry.perf[cpu].cycles as f64;
            count += 1;
        }
    }
    total / count as f64
}

const UNIT_NAMES: &[&str] = &[" ", ",000", "M", "B", "T"];
fn format_number(number: f64) -> String {
    let mut number = number;
    let mut unit = 0;
    while number >= 1000.0 {
        number /= 1000.0;
        unit += 1;
    }
    format!("{:.2}{}", number, UNIT_NAMES[unit])
}

fn print_string_collection(arr: &[String]) {
    let (width, _) = termion::terminal_size().unwrap_or((u16::MAX, u16::MAX));
    let width = width as i32;
    let mut remaining_width = width as i32;
    let mut first = true;
    let max_len = arr.iter().map(|x| x.len()).max().unwrap();
    for item in arr {
        let space = " | ";
        remaining_width -= (max_len + space.len()) as i32;
        if first || remaining_width < 0 {
            if !first {
                println!();
            }
            print!("{:max_len$}", item);
            remaining_width = width - max_len as i32;
            first = false;
        } else {
            print!("{}{:max_len$}", space, item);
        }
    }
    println!();
}

fn main() {
    let mut args = std::env::args();
    let cmd = args.next().unwrap();
    if args.len() == 0 {
        println!("Usage: {} <file1> <file2> ...", cmd);
        exit(1);
    }

    for filename in args {
        let report = get_report(std::fs::read_to_string(&filename).unwrap());

        println!("{:20} {}", "Report ID ".bold().yellow(), report.id);
        println!("{:20} {}", "File ".bold().yellow(), filename);
        println!(
            "{:20} {}",
            "Test Duration ".bold().yellow(),
            report.duration.as_secs()
        );
        println!(
            "{:20} {}",
            "Test Interval ".bold().yellow(),
            report.interval.as_secs()
        );

        println!("{}", "Per CPU average load".bold().yellow());
        let max_cpu_len = report.proc_cpus.iter().map(|s| s.len()).max().unwrap_or(0);
        let proc_average_load = report
            .proc_cpus
            .iter()
            .map(|cpu| {
                format!(
                    "{:max_cpu_len$} {:>6.20}%",
                    format!("{}", cpu).bold().yellow(),
                    format_number(get_average_proc_load(&report, cpu))
                )
            })
            .collect::<Vec<_>>();
        print_string_collection(&proc_average_load);
        println!("{}", "Per CPU average CPU cycles".bold().yellow());
        let max_cpu_len = report.perf_cpus.iter().map(|s| s.len()).max().unwrap_or(0);
        let perf_average_cycles = report
            .perf_cpus
            .iter()
            .map(|cpu| {
                format!(
                    "{:max_cpu_len$} {}",
                    format!("{}", cpu).bold().yellow(),
                    format_number(get_average_cpu_cycles(&report, cpu))
                )
            })
            .collect::<Vec<_>>();
        print_string_collection(&perf_average_cycles);
        println!();
    }
}
