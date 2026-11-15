//! Command-line parsing and the (stubbed) serving entry point.

use std::fmt;
use std::path::PathBuf;

use crate::mirror::Mirror;
use crate::sim::SimAdapter;
use crate::table::{MirrorError, apply_name};

/// The usage text.
pub const USAGE: &str = "\
usage: gattmirror (--simulate | --profile <file>) [--hci <host:port>] [--name <name>]

  --simulate         mirror the built-in simulated serial adapter
  --profile <file>   mirror the peripheral described by a gatt-profile text file
  --hci <host:port>  the virtual controller's HCI transport to serve on
  --name <name>      advertise and serve this device name instead of the profile's
  -h, --help         show this text
";

/// A host and port, as given to `--hci`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostPort {
    /// Host name or address.
    pub host: String,
    /// TCP port.
    pub port: u16,
}

/// Parsed arguments.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Args {
    /// Use the simulated adapter.
    pub simulate: bool,
    /// A profile file to mirror.
    pub profile: Option<PathBuf>,
    /// The virtual controller's HCI transport.
    pub hci: Option<HostPort>,
    /// A device name override.
    pub name: Option<String>,
}

/// What the command line asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Print [`USAGE`].
    Help,
    /// Run with these arguments.
    Run(Args),
}

/// A command line that cannot be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArgsError {
    /// An option nobody defined.
    Unknown(String),
    /// An option that takes a value was last.
    MissingValue(&'static str),
    /// `--hci` was not `host:port` with a numeric port.
    BadHci(String),
    /// An option was given twice.
    Repeated(&'static str),
    /// Neither `--simulate` nor `--profile` was given.
    NoSource,
    /// Both `--simulate` and `--profile` were given.
    TwoSources,
}

impl fmt::Display for ArgsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(a) => write!(f, "unknown argument {a:?}"),
            Self::MissingValue(o) => write!(f, "{o} needs a value"),
            Self::BadHci(v) => write!(f, "--hci wants host:port, got {v:?}"),
            Self::Repeated(o) => write!(f, "{o} given twice"),
            Self::NoSource => f.write_str("give --simulate or --profile <file>"),
            Self::TwoSources => f.write_str("--simulate and --profile are alternatives"),
        }
    }
}

impl std::error::Error for ArgsError {}

fn hci(value: &str) -> Result<HostPort, ArgsError> {
    let bad = || ArgsError::BadHci(value.to_owned());
    let (host, port) = value.rsplit_once(':').ok_or_else(bad)?;
    if host.is_empty() {
        return Err(bad());
    }
    let port = port.parse().map_err(|_| bad())?;
    Ok(HostPort {
        host: host.to_owned(),
        port,
    })
}

/// Parse the arguments after the program name.
///
/// # Errors
/// [`ArgsError`] describing the first problem.
pub fn parse<I, S>(args: I) -> Result<Command, ArgsError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut it = args.into_iter().map(Into::into);
    let mut out = Args::default();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            "--simulate" => {
                if out.simulate {
                    return Err(ArgsError::Repeated("--simulate"));
                }
                out.simulate = true;
            }
            "--profile" => {
                let v = it.next().ok_or(ArgsError::MissingValue("--profile"))?;
                if out.profile.replace(PathBuf::from(v)).is_some() {
                    return Err(ArgsError::Repeated("--profile"));
                }
            }
            "--hci" => {
                let v = it.next().ok_or(ArgsError::MissingValue("--hci"))?;
                if out.hci.replace(hci(&v)?).is_some() {
                    return Err(ArgsError::Repeated("--hci"));
                }
            }
            "--name" => {
                let v = it.next().ok_or(ArgsError::MissingValue("--name"))?;
                if out.name.replace(v).is_some() {
                    return Err(ArgsError::Repeated("--name"));
                }
            }
            _ => return Err(ArgsError::Unknown(a)),
        }
    }
    match (out.simulate, out.profile.is_some()) {
        (false, false) => Err(ArgsError::NoSource),
        (true, true) => Err(ArgsError::TwoSources),
        _ => Ok(Command::Run(out)),
    }
}

/// Why [`run`] stopped.
#[derive(Debug)]
pub enum RunError {
    /// The profile file could not be read.
    Read(PathBuf, std::io::Error),
    /// The profile file did not parse.
    Parse(gatt_profile::ParseError),
    /// The mirror could not be built.
    Mirror(MirrorError),
    /// `--profile` names a peripheral but no real-adapter backend exists yet.
    NoAdapterBackend,
    /// The mirror is built but nothing serves it: the connection API is not wired in.
    ServingNotWired,
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(p, e) => write!(f, "cannot read {}: {e}", p.display()),
            Self::Parse(e) => write!(f, "cannot parse the profile: {e:?}"),
            Self::Mirror(e) => e.fmt(f),
            Self::NoAdapterBackend => {
                f.write_str("no real-adapter backend yet (winrt-gatt is not wired in); use --simulate")
            }
            Self::ServingNotWired => f.write_str(
                "the table is built, but serving it needs the le-host connection API, which is not wired in yet",
            ),
        }
    }
}

impl std::error::Error for RunError {}

/// Build the mirror the arguments ask for and describe it on `out`.
///
/// **Stub**: this does not yet open the `--hci` transport or accept a connection. It returns
/// [`RunError::ServingNotWired`] after building the table, so a bad profile still fails here.
///
/// # Errors
/// [`RunError`].
pub fn run(args: &Args, out: &mut dyn std::io::Write) -> Result<(), RunError> {
    if !args.simulate {
        let path = args.profile.clone().unwrap_or_default();
        let text = std::fs::read_to_string(&path).map_err(|e| RunError::Read(path.clone(), e))?;
        let mut profile = gatt_profile::parse(&text).map_err(RunError::Parse)?;
        if let Some(n) = &args.name {
            apply_name(&mut profile, n);
        }
        crate::table::build_database(&profile).map_err(RunError::Mirror)?;
        return Err(RunError::NoAdapterBackend);
    }
    let mut adapter = SimAdapter::new();
    if let Some(n) = &args.name {
        adapter.set_name(n);
    }
    let mirror = Mirror::new(adapter, 247).map_err(RunError::Mirror)?;
    let attrs = mirror.server().database().attributes().len();
    let _ = writeln!(out, "mirroring the simulated adapter: {attrs} attributes");
    if let Some(h) = &args.hci {
        let _ = writeln!(out, "hci transport: {}:{}", h.host, h.port);
    }
    Err(RunError::ServingNotWired)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_args(a: &[&str]) -> Result<Command, ArgsError> {
        parse(a.iter().copied())
    }

    fn args(a: &[&str]) -> Args {
        match run_args(a).unwrap() {
            Command::Run(a) => a,
            Command::Help => panic!("help"),
        }
    }

    #[test]
    fn simulate_alone_is_enough() {
        assert_eq!(
            args(&["--simulate"]),
            Args {
                simulate: true,
                ..Args::default()
            }
        );
    }

    #[test]
    fn every_option_parses() {
        let a = args(&[
            "--profile",
            "p.gatt",
            "--hci",
            "10.0.2.2:5001",
            "--name",
            "My Dev",
        ]);
        assert_eq!(a.profile, Some(PathBuf::from("p.gatt")));
        assert_eq!(
            a.hci,
            Some(HostPort {
                host: "10.0.2.2".into(),
                port: 5001
            })
        );
        assert_eq!(a.name.as_deref(), Some("My Dev"));
        assert!(!a.simulate);
    }

    #[test]
    fn help_wins() {
        assert_eq!(run_args(&["--simulate", "-h"]), Ok(Command::Help));
    }

    #[test]
    fn a_source_is_required_and_only_one() {
        assert_eq!(run_args(&[]), Err(ArgsError::NoSource));
        assert_eq!(run_args(&["--name", "x"]), Err(ArgsError::NoSource));
        assert_eq!(
            run_args(&["--simulate", "--profile", "p"]),
            Err(ArgsError::TwoSources)
        );
    }

    #[test]
    fn bad_hci_values_are_refused() {
        for v in ["host", ":5000", "h:", "h:99999", "h:x"] {
            assert_eq!(
                run_args(&["--simulate", "--hci", v]),
                Err(ArgsError::BadHci(v.into())),
                "{v}"
            );
        }
    }

    #[test]
    fn missing_unknown_and_repeated_options() {
        assert_eq!(
            run_args(&["--simulate", "--name"]),
            Err(ArgsError::MissingValue("--name"))
        );
        assert_eq!(
            run_args(&["--simulate", "--bogus"]),
            Err(ArgsError::Unknown("--bogus".into()))
        );
        assert_eq!(
            run_args(&["--simulate", "--simulate"]),
            Err(ArgsError::Repeated("--simulate"))
        );
        assert_eq!(
            run_args(&["--simulate", "--name", "a", "--name", "b"]),
            Err(ArgsError::Repeated("--name"))
        );
    }

    #[test]
    fn simulate_run_builds_then_reports_serving_is_not_wired() {
        let mut out = Vec::new();
        let e = run(&args(&["--simulate", "--hci", "h:1"]), &mut out).unwrap_err();
        assert!(matches!(e, RunError::ServingNotWired));
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("attributes"), "{text}");
        assert!(text.contains("hci transport: h:1"), "{text}");
    }

    #[test]
    fn a_missing_profile_file_is_a_read_error() {
        let a = args(&["--profile", "does-not-exist.gatt"]);
        let e = run(&a, &mut Vec::new()).unwrap_err();
        assert!(matches!(e, RunError::Read(..)));
    }
}
