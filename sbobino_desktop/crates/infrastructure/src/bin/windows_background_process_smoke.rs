#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
fn main() {
    use sbobino_infrastructure::background_process::std_background_command;

    let mut arguments = std::env::args_os().skip(1);
    let program = arguments
        .next()
        .expect("usage: windows_background_process_smoke.exe <program> [arguments ...]");
    let program_arguments = arguments.collect::<Vec<_>>();

    for _ in 0..20 {
        std_background_command(&program)
            .args(&program_arguments)
            .status()
            .expect("failed to launch a real Windows background helper");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("windows-background-process-smoke is only meaningful on Windows");
}
