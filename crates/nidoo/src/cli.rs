use std::path::PathBuf;

use crate::migrate::{MigrateArgs, Mode};
use clap::parser::ValueSource;
use clap::{ArgMatches, CommandFactory, FromArgMatches, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "nidoo", version, about = "Nidoo home-automation hub")]
struct RawCli {
    #[command(subcommand)]
    command: Option<RawCommand>,
    /// Config file used when no subcommand is given (same as `run --config`); `init-bundled`
    /// treats an existing one as a hub that is already set up.
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,
    /// Folder holding `hub.json` and the settings cache (`OIKOS_DATA_DIR` is still read when
    /// `NIDOO_DATA_DIR` is not set).
    #[arg(
        long = "data",
        global = true,
        value_name = "DIR",
        env = "NIDOO_DATA_DIR",
        default_value = "./data"
    )]
    data_dir: PathBuf,
}

#[derive(Debug, Subcommand)]
enum RawCommand {
    /// Run the hub (default).
    Run,
    /// Serve MCP over stdio, proxying to a running hub.
    McpStdio {
        /// Base URL of the hub, e.g. http://localhost:8081
        #[arg(long)]
        server: String,
    },
    /// Create the bundled broker and database credentials, then exit.
    InitBundled,
    /// One-off: move the `oikos` retained messages, database rows, database and role to
    /// `nidoo`. Without a flag it is a dry run that writes nothing.
    #[command(name = "migrate-oikos")]
    Migrate {
        /// Do the switch-over (the hub must be stopped; needs `--backup`).
        #[arg(long, conflicts_with_all = ["verify", "restore"], requires = "backup")]
        apply: bool,
        /// Report anything still carrying the old name; exits non-zero when something does.
        #[arg(long, conflicts_with = "restore")]
        verify: bool,
        /// Write the retained messages to this file first.
        #[arg(long, value_name = "FILE", conflicts_with = "restore")]
        backup: Option<PathBuf>,
        /// Republish the retained messages of a backup file, then exit.
        #[arg(long, value_name = "FILE")]
        restore: Option<PathBuf>,
    },
}

/// Resolved command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cli {
    Run {
        config: Option<PathBuf>,
        data_dir: PathBuf,
    },
    McpStdio {
        server: String,
        data_dir: PathBuf,
    },
    InitBundled {
        data_dir: PathBuf,
        config: Option<PathBuf>,
    },
    Migrate {
        args: MigrateArgs,
        data_dir: PathBuf,
    },
}

impl Cli {
    pub fn parse_args() -> Cli {
        Self::try_parse_from(std::env::args_os()).unwrap_or_else(|e| e.exit())
    }

    pub fn try_parse_from<I, T>(args: I) -> Result<Cli, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        let matches = RawCli::command().try_get_matches_from(args)?;
        let mut raw = RawCli::from_arg_matches(&matches)?;
        if let Some(dir) = legacy_data_dir(&matches) {
            raw.data_dir = dir;
        }
        Ok(Self::from_raw(raw))
    }

    fn from_raw(raw: RawCli) -> Cli {
        match raw.command {
            None | Some(RawCommand::Run) => Cli::Run {
                config: raw.config,
                data_dir: raw.data_dir,
            },
            Some(RawCommand::McpStdio { server }) => Cli::McpStdio {
                server,
                data_dir: raw.data_dir,
            },
            Some(RawCommand::InitBundled) => Cli::InitBundled {
                data_dir: raw.data_dir,
                config: raw.config,
            },
            Some(RawCommand::Migrate {
                apply,
                verify,
                backup,
                restore,
            }) => Cli::Migrate {
                args: MigrateArgs {
                    mode: if apply {
                        Mode::Apply
                    } else if verify {
                        Mode::Verify
                    } else {
                        Mode::DryRun
                    },
                    backup,
                    restore,
                },
                data_dir: raw.data_dir,
            },
        }
    }
}

/// `OIKOS_DATA_DIR`, when the data folder came from neither `--data` nor `NIDOO_DATA_DIR`.
fn legacy_data_dir(matches: &ArgMatches) -> Option<PathBuf> {
    if matches.value_source("data_dir") != Some(ValueSource::DefaultValue) {
        return None;
    }
    std::env::var_os("OIKOS_DATA_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::result_large_err)] // figment::Jail closures return figment::Error

    use super::*;

    fn run(config: Option<&str>, data: &str) -> Cli {
        Cli::Run {
            config: config.map(Into::into),
            data_dir: data.into(),
        }
    }

    // Each parse runs inside a Jail, which serializes it with the tests that set env vars.
    #[test]
    fn run_is_default() {
        figment::Jail::expect_with(|_| {
            assert_eq!(Cli::try_parse_from(["nidoo"]).unwrap(), run(None, "./data"));
            assert_eq!(
                Cli::try_parse_from(["nidoo", "run", "--config", "x.toml"]).unwrap(),
                run(Some("x.toml"), "./data")
            );
            assert_eq!(
                Cli::try_parse_from(["nidoo", "--config", "y.toml"]).unwrap(),
                run(Some("y.toml"), "./data")
            );
            Ok(())
        });
    }

    #[test]
    fn data_defaults_to_dot_data() {
        figment::Jail::expect_with(|_| {
            assert_eq!(Cli::try_parse_from(["nidoo"]).unwrap(), run(None, "./data"));
            Ok(())
        });
    }

    #[test]
    fn data_flag_and_env() {
        figment::Jail::expect_with(|jail| {
            assert_eq!(
                Cli::try_parse_from(["nidoo", "--data", "/srv/oikos"]).unwrap(),
                run(None, "/srv/oikos")
            );
            assert_eq!(
                Cli::try_parse_from(["nidoo", "run", "--data", "d"]).unwrap(),
                run(None, "d")
            );
            jail.set_env("OIKOS_DATA_DIR", "/data");
            assert_eq!(Cli::try_parse_from(["nidoo"]).unwrap(), run(None, "/data"));
            // The flag beats the environment.
            assert_eq!(
                Cli::try_parse_from(["nidoo", "--data", "x"]).unwrap(),
                run(None, "x")
            );
            Ok(())
        });
    }

    #[test]
    fn data_dir_falls_back_to_oikos_env() {
        figment::Jail::expect_with(|jail| {
            jail.set_env("OIKOS_DATA_DIR", "/data");
            assert_eq!(Cli::try_parse_from(["nidoo"]).unwrap(), run(None, "/data"));
            // An explicit flag still beats the legacy variable.
            assert_eq!(
                Cli::try_parse_from(["nidoo", "run", "--data", "d"]).unwrap(),
                run(None, "d")
            );
            jail.set_env("NIDOO_DATA_DIR", "/n");
            assert_eq!(Cli::try_parse_from(["nidoo"]).unwrap(), run(None, "/n"));
            Ok(())
        });
    }

    #[test]
    fn mcp_stdio_takes_server_and_data() {
        figment::Jail::expect_with(|_| {
            assert_eq!(
                Cli::try_parse_from(["nidoo", "mcp-stdio", "--server", "http://h:1"]).unwrap(),
                Cli::McpStdio {
                    server: "http://h:1".into(),
                    data_dir: "./data".into(),
                }
            );
            assert_eq!(
                Cli::try_parse_from([
                    "nidoo",
                    "--data",
                    "d",
                    "mcp-stdio",
                    "--server",
                    "http://h:1"
                ])
                .unwrap(),
                Cli::McpStdio {
                    server: "http://h:1".into(),
                    data_dir: "d".into(),
                }
            );
            Ok(())
        });
    }

    #[test]
    fn init_bundled_takes_data() {
        figment::Jail::expect_with(|_| {
            assert_eq!(
                Cli::try_parse_from(["nidoo", "init-bundled"]).unwrap(),
                Cli::InitBundled {
                    data_dir: "./data".into(),
                    config: None,
                }
            );
            assert_eq!(
                Cli::try_parse_from(["nidoo", "--data", "/data", "init-bundled"]).unwrap(),
                Cli::InitBundled {
                    data_dir: "/data".into(),
                    config: None,
                }
            );
            // `--config` parses before or after the subcommand.
            for args in [
                ["nidoo", "--config", "/checkout/oikos.toml", "init-bundled"],
                ["nidoo", "init-bundled", "--config", "/checkout/oikos.toml"],
            ] {
                assert_eq!(
                    Cli::try_parse_from(args).unwrap(),
                    Cli::InitBundled {
                        data_dir: "./data".into(),
                        config: Some("/checkout/oikos.toml".into()),
                    }
                );
            }
            Ok(())
        });
    }

    #[test]
    fn migrate_defaults_to_dry_run() {
        figment::Jail::expect_with(|_| {
            assert_eq!(
                Cli::try_parse_from(["nidoo", "migrate-oikos"]).unwrap(),
                Cli::Migrate {
                    args: MigrateArgs {
                        mode: Mode::DryRun,
                        backup: None,
                        restore: None,
                    },
                    data_dir: "./data".into(),
                }
            );
            assert_eq!(
                Cli::try_parse_from(["nidoo", "--data", "d", "migrate-oikos", "--verify"]).unwrap(),
                Cli::Migrate {
                    args: MigrateArgs {
                        mode: Mode::Verify,
                        backup: None,
                        restore: None,
                    },
                    data_dir: "d".into(),
                }
            );
            Ok(())
        });
    }

    #[test]
    fn migrate_apply_requires_backup() {
        figment::Jail::expect_with(|_| {
            assert!(Cli::try_parse_from(["nidoo", "migrate-oikos", "--apply"]).is_err());
            assert_eq!(
                Cli::try_parse_from(["nidoo", "migrate-oikos", "--apply", "--backup", "b.jsonl"])
                    .unwrap(),
                Cli::Migrate {
                    args: MigrateArgs {
                        mode: Mode::Apply,
                        backup: Some("b.jsonl".into()),
                        restore: None,
                    },
                    data_dir: "./data".into(),
                }
            );
            assert!(
                Cli::try_parse_from([
                    "nidoo",
                    "migrate-oikos",
                    "--apply",
                    "--verify",
                    "--backup",
                    "b"
                ])
                .is_err()
            );
            Ok(())
        });
    }

    #[test]
    fn migrate_restore_takes_file() {
        figment::Jail::expect_with(|_| {
            assert_eq!(
                Cli::try_parse_from(["nidoo", "migrate-oikos", "--restore", "b.jsonl"]).unwrap(),
                Cli::Migrate {
                    args: MigrateArgs {
                        mode: Mode::DryRun,
                        backup: None,
                        restore: Some("b.jsonl".into()),
                    },
                    data_dir: "./data".into(),
                }
            );
            assert!(
                Cli::try_parse_from(["nidoo", "migrate-oikos", "--restore", "b", "--apply"])
                    .is_err()
            );
            Ok(())
        });
    }
}
