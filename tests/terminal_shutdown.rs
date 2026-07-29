#![cfg(unix)]

use std::{
    ffi::OsStr,
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    os::unix::ffi::OsStrExt,
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    thread,
    time::{Duration, Instant},
};

use rustix::{
    pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt},
    termios::{LocalModes, Winsize, tcgetattr, tcsetwinsize},
};
use stock_tui::terminal::TerminalSession;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(20);
const EXIT_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_SHUTDOWN_LATENCY: Duration = Duration::from_millis(750);
const SHELL_READ_TIMEOUT: Duration = Duration::from_secs(1);

#[test]
fn pending_mouse_reports_are_drained_before_shell_restore() -> Result<(), Box<dyn std::error::Error>>
{
    let temp = tempfile::tempdir()?;
    let (slave, mut master_writer, output_rx, output_thread) = open_test_pty()?;

    let mut child = Command::new(env!("CARGO_BIN_EXE_stock-tui"))
        .args(["--offline", "--db"])
        .arg(temp.path().join("terminal-shutdown.sqlite3"))
        .env("HOME", temp.path())
        .env("XDG_CACHE_HOME", temp.path().join("cache"))
        .env("XDG_CONFIG_HOME", temp.path().join("config"))
        .env("XDG_DATA_HOME", temp.path().join("data"))
        .env("TERM", "xterm-256color")
        .stdin(Stdio::from(slave.try_clone()?))
        .stdout(Stdio::from(slave.try_clone()?))
        .stderr(Stdio::from(slave.try_clone()?))
        .spawn()?;

    let mut output = Vec::new();
    receive_until(
        &output_rx,
        &mut output,
        b"\x1b[?1003h\x1b[?1006h",
        STARTUP_TIMEOUT,
    )?;

    let shutdown_started = Instant::now();
    master_writer.write_all(b"q\x1b[<35;11;7M")?;
    master_writer.flush()?;
    receive_until(
        &output_rx,
        &mut output,
        b"\x1b[?1006l\x1b[?1003l",
        Duration::from_secs(1),
    )?;
    master_writer.write_all(b"\x1b[<35;70;20M")?;
    master_writer.flush()?;

    let status = wait_for_exit(&mut child, EXIT_TIMEOUT)?;
    assert!(status.success(), "stock-tui exited with {status}");
    assert!(
        shutdown_started.elapsed() < MAX_SHUTDOWN_LATENCY,
        "terminal shutdown exceeded {MAX_SHUTDOWN_LATENCY:?}"
    );

    receive_until(
        &output_rx,
        &mut output,
        b"\x1b[?1049l",
        Duration::from_secs(1),
    )?;
    let disable_position = find_bytes(&output, b"\x1b[?1006l\x1b[?1003l")
        .expect("mouse disable sequence must be present");
    let leave_position =
        find_bytes(&output, b"\x1b[?1049l").expect("alternate-screen leave must be present");
    assert!(
        disable_position < leave_position,
        "mouse reporting must stop before leaving the alternate screen"
    );

    let terminal_state = tcgetattr(&slave)?;
    assert!(
        terminal_state.local_modes.contains(LocalModes::ICANON),
        "canonical input was not restored"
    );
    assert!(
        terminal_state.local_modes.contains(LocalModes::ECHO),
        "terminal echo was not restored"
    );

    assert_clean_shell_read(&slave, &mut master_writer, b"shell-ready\n")?;

    drop(slave);
    drop(master_writer);
    output_thread
        .join()
        .expect("the terminal output reader must not panic");
    Ok(())
}

#[test]
fn demo_seed_is_cancelled_before_terminal_restore() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let (slave, mut master_writer, output_rx, output_thread) = open_test_pty()?;
    let mut child = Command::new(env!("CARGO_BIN_EXE_stock-tui"))
        .args(["--demo", "--db"])
        .arg(temp.path().join("demo-shutdown.sqlite3"))
        .env("HOME", temp.path())
        .env("XDG_CACHE_HOME", temp.path().join("cache"))
        .env("XDG_CONFIG_HOME", temp.path().join("config"))
        .env("XDG_DATA_HOME", temp.path().join("data"))
        .env("TERM", "xterm-256color")
        .stdin(Stdio::from(slave.try_clone()?))
        .stdout(Stdio::from(slave.try_clone()?))
        .stderr(Stdio::from(slave.try_clone()?))
        .spawn()?;

    let mut output = Vec::new();
    receive_until(
        &output_rx,
        &mut output,
        b"\x1b[?1003h\x1b[?1006h",
        STARTUP_TIMEOUT,
    )?;
    let shutdown_started = Instant::now();
    master_writer.write_all(b"q\x1b[<35;15;9M")?;
    master_writer.flush()?;
    receive_until(
        &output_rx,
        &mut output,
        b"\x1b[?1006l\x1b[?1003l",
        Duration::from_secs(1),
    )?;
    master_writer.write_all(b"\x1b[<35;85;30M")?;
    master_writer.flush()?;

    let status = wait_for_exit(&mut child, EXIT_TIMEOUT)?;
    assert!(status.success(), "demo stock-tui exited with {status}");
    assert!(
        shutdown_started.elapsed() < EXIT_TIMEOUT,
        "demo shutdown exceeded {EXIT_TIMEOUT:?}"
    );
    receive_until(
        &output_rx,
        &mut output,
        b"\x1b[?1049l",
        Duration::from_secs(1),
    )?;

    let terminal_state = tcgetattr(&slave)?;
    assert!(
        terminal_state.local_modes.contains(LocalModes::ICANON),
        "demo shutdown did not restore canonical input"
    );
    assert!(
        terminal_state.local_modes.contains(LocalModes::ECHO),
        "demo shutdown did not restore terminal echo"
    );
    assert_clean_shell_read(&slave, &mut master_writer, b"shell-after-demo\n")?;

    drop(slave);
    drop(master_writer);
    output_thread
        .join()
        .expect("the terminal output reader must not panic");
    Ok(())
}

#[test]
fn panic_cleanup_restores_modes_and_discards_buffered_input()
-> Result<(), Box<dyn std::error::Error>> {
    let (slave, mut master_writer, output_rx, output_thread) = open_test_pty()?;
    let mut child = Command::new(std::env::current_exe()?)
        .args(["--ignored", "--exact", "panic_cleanup_child", "--nocapture"])
        .env("STOCK_TUI_PANIC_CHILD", "1")
        .env("TERM", "xterm-256color")
        .stdin(Stdio::from(slave.try_clone()?))
        .stdout(Stdio::from(slave.try_clone()?))
        .stderr(Stdio::from(slave.try_clone()?))
        .spawn()?;

    let mut output = Vec::new();
    receive_until(
        &output_rx,
        &mut output,
        b"\x1b[?1003h\x1b[?1006h",
        STARTUP_TIMEOUT,
    )?;
    master_writer.write_all(b"x\x1b[<35;41;12M")?;
    master_writer.flush()?;

    let status = wait_for_exit(&mut child, EXIT_TIMEOUT)?;
    assert!(!status.success(), "the panic helper unexpectedly succeeded");
    receive_until(
        &output_rx,
        &mut output,
        b"\x1b[?1006l\x1b[?1003l",
        Duration::from_secs(1),
    )?;
    receive_until(
        &output_rx,
        &mut output,
        b"\x1b[?1049l",
        Duration::from_secs(1),
    )?;
    receive_until(
        &output_rx,
        &mut output,
        b"\x1b[?25h",
        Duration::from_secs(1),
    )?;

    let terminal_state = tcgetattr(&slave)?;
    assert!(
        terminal_state.local_modes.contains(LocalModes::ICANON),
        "panic cleanup did not restore canonical input"
    );
    assert!(
        terminal_state.local_modes.contains(LocalModes::ECHO),
        "panic cleanup did not restore terminal echo"
    );

    assert_clean_shell_read(&slave, &mut master_writer, b"shell-after-panic\n")?;
    drop(slave);
    drop(master_writer);
    output_thread
        .join()
        .expect("the terminal output reader must not panic");
    Ok(())
}

#[test]
#[ignore = "spawned by panic_cleanup_restores_modes_and_discards_buffered_input"]
fn panic_cleanup_child() {
    if std::env::var_os("STOCK_TUI_PANIC_CHILD").is_none() {
        return;
    }

    let _session = TerminalSession::start().expect("the panic helper terminal must start");
    let mut trigger = [0_u8; 1];
    io::stdin()
        .read_exact(&mut trigger)
        .expect("the panic trigger must arrive");
    panic!("intentional terminal cleanup test panic");
}

type PtyParts = (File, File, Receiver<Vec<u8>>, thread::JoinHandle<()>);

fn open_test_pty() -> Result<PtyParts, Box<dyn std::error::Error>> {
    let master_fd = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY)?;
    grantpt(&master_fd)?;
    unlockpt(&master_fd)?;
    let slave_name = ptsname(&master_fd, Vec::new())?;
    let slave_path = Path::new(OsStr::from_bytes(slave_name.to_bytes()));
    let slave = OpenOptions::new().read(true).write(true).open(slave_path)?;
    tcsetwinsize(
        &slave,
        Winsize {
            ws_row: 42,
            ws_col: 140,
            ws_xpixel: 0,
            ws_ypixel: 0,
        },
    )?;

    let master_reader: File = master_fd.into();
    let master_writer = master_reader.try_clone()?;
    let (output_tx, output_rx) = mpsc::channel();
    let output_thread = thread::spawn(move || {
        let mut reader = master_reader;
        let mut buffer = [0_u8; 8_192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    if output_tx.send(buffer[..count].to_vec()).is_err() {
                        break;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
    });
    Ok((slave, master_writer, output_rx, output_thread))
}

fn assert_clean_shell_read(
    slave: &File,
    master_writer: &mut File,
    expected: &[u8],
) -> io::Result<()> {
    let mut shell_input = slave.try_clone()?;
    let (shell_tx, shell_rx) = mpsc::channel();
    let shell_reader = thread::spawn(move || {
        let mut line = [0_u8; 256];
        let result = shell_input
            .read(&mut line)
            .map(|count| line[..count].to_vec());
        let _ = shell_tx.send(result);
    });
    master_writer.write_all(expected)?;
    master_writer.flush()?;
    let shell_line = shell_rx
        .recv_timeout(SHELL_READ_TIMEOUT)
        .map_err(|error| io::Error::new(io::ErrorKind::TimedOut, error))??;
    assert_eq!(
        shell_line, expected,
        "the next shell reader received stale terminal input"
    );
    shell_reader
        .join()
        .expect("the simulated shell reader must not panic");
    Ok(())
}

fn wait_for_exit(
    child: &mut Child,
    timeout: Duration,
) -> Result<std::process::ExitStatus, Box<dyn std::error::Error>> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            child.kill()?;
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("stock-tui did not exit within {timeout:?}"),
            )
            .into());
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn receive_until(
    receiver: &Receiver<Vec<u8>>,
    output: &mut Vec<u8>,
    needle: &[u8],
    timeout: Duration,
) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    while find_bytes(output, needle).is_none() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("terminal output did not contain {needle:?}"),
            ));
        }
        match receiver.recv_timeout(remaining) {
            Ok(chunk) => output.extend_from_slice(&chunk),
            Err(RecvTimeoutError::Timeout) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("terminal output did not contain {needle:?}"),
                ));
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!("terminal output ended before {needle:?}"),
                ));
            }
        }
    }
    Ok(())
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
