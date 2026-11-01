use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "oikos-os", version, about = "Nidoo home-automation hub")]
struct RawCli {
    #[command(subcommand)]
    command: Option<RawCommand>,
    /// Config file used when no subcommand is given (same as `run --config`); `init-bundled`
    /// treats an existing one as a hub that is already set up.
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,
    /// Folder holding `hub.json` and the settings cache.
    #[arg(
        long = "data",
        global = true,
        value_name = "DIR",
        env = "OIKOS_DATA_DIR",
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
}

impl Cli {
    pub fn parse_args() -> Cli {
        Self::from_raw(RawCli::parse())
    }

    pub fn try_parse_from<I, T>(args: I) -> Result<Cli, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        RawCli::try_parse_from(args).map(Self::from_raw)
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
        }
    }
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
            assert_eq!(
                Cli::try_parse_from(["oikos-os"]).unwrap(),
                run(None, "./data")
            );
            assert_eq!(
                Cli::try_parse_from(["oikos-os", "run", "--config", "x.toml"]).unwrap(),
                run(Some("x.toml"), "./data")
            );
            assert_eq!(
                Cli::try_parse_from(["oikos-os", "--config", "y.toml"]).unwrap(),
                run(Some("y.toml"), "./data")
            );
            Ok(())
        });
    }

    #[test]
    fn data_defaults_to_dot_data() {
        figment::Jail::expect_with(|_| {
            assert_eq!(
                Cli::try_parse_from(["oikos-os"]).unwrap(),
                run(None, "./data")
            );
            Ok(())
        });
    }

    #[test]
    fn data_flag_and_env() {
        figment::Jail::expect_with(|jail| {
            assert_eq!(
                Cli::try_parse_from(["oikos-os", "--data", "/srv/oikos"]).unwrap(),
                run(None, "/srv/oikos")
            );
            assert_eq!(
                Cli::try_parse_from(["oikos-os", "run", "--data", "d"]).unwrap(),
                run(None, "d")
            );
            jail.set_env("OIKOS_DATA_DIR", "/data");
            assert_eq!(
                Cli::try_parse_from(["oikos-os"]).unwrap(),
                run(None, "/data")
            );
            // The flag beats the environment.
            assert_eq!(
                Cli::try_parse_from(["oikos-os", "--data", "x"]).unwrap(),
                run(None, "x")
            );
            Ok(())
        });
    }

    #[test]
    fn mcp_stdio_takes_server_and_data() {
        figment::Jail::expect_with(|_| {
            assert_eq!(
                Cli::try_parse_from(["oikos-os", "mcp-stdio", "--server", "http://h:1"]).unwrap(),
                Cli::McpStdio {
                    server: "http://h:1".into(),
                    data_dir: "./data".into(),
                }
            );
            assert_eq!(
                Cli::try_parse_from([
                    "oikos-os",
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
                Cli::try_parse_from(["oikos-os", "init-bundled"]).unwrap(),
                Cli::InitBundled {
                    data_dir: "./data".into(),
                    config: None,
                }
            );
            assert_eq!(
                Cli::try_parse_from(["oikos-os", "--data", "/data", "init-bundled"]).unwrap(),
                Cli::InitBundled {
                    data_dir: "/data".into(),
                    config: None,
                }
            );
            // `--config` parses before or after the subcommand.
            for args in [
                [
                    "oikos-os",
                    "--config",
                    "/checkout/oikos.toml",
                    "init-bundled",
                ],
                [
                    "oikos-os",
                    "init-bundled",
                    "--config",
                    "/checkout/oikos.toml",
                ],
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
}
