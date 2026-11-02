use std::path::PathBuf;

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "nidoo", version, about = "Nidoo home-automation hub")]
struct RawCli {
    #[command(subcommand)]
    command: Option<RawCommand>,
    /// Folder holding `hub.json` and the settings cache.
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
}

/// Resolved command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cli {
    Run { data_dir: PathBuf },
    McpStdio { server: String, data_dir: PathBuf },
    InitBundled { data_dir: PathBuf },
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
        let raw = RawCli::from_arg_matches(&matches)?;
        Ok(Self::from_raw(raw))
    }

    fn from_raw(raw: RawCli) -> Cli {
        match raw.command {
            None | Some(RawCommand::Run) => Cli::Run {
                data_dir: raw.data_dir,
            },
            Some(RawCommand::McpStdio { server }) => Cli::McpStdio {
                server,
                data_dir: raw.data_dir,
            },
            Some(RawCommand::InitBundled) => Cli::InitBundled {
                data_dir: raw.data_dir,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::result_large_err)] // figment::Jail closures return figment::Error

    use super::*;

    fn run(data: &str) -> Cli {
        Cli::Run {
            data_dir: data.into(),
        }
    }

    // Each parse runs inside a Jail, which serializes it with the tests that set env vars.
    #[test]
    fn run_is_default() {
        figment::Jail::expect_with(|_| {
            assert_eq!(Cli::try_parse_from(["nidoo"]).unwrap(), run("./data"));
            assert_eq!(
                Cli::try_parse_from(["nidoo", "run"]).unwrap(),
                run("./data")
            );
            Ok(())
        });
    }

    #[test]
    fn data_defaults_to_dot_data() {
        figment::Jail::expect_with(|_| {
            assert_eq!(Cli::try_parse_from(["nidoo"]).unwrap(), run("./data"));
            Ok(())
        });
    }

    #[test]
    fn data_flag_and_env() {
        figment::Jail::expect_with(|jail| {
            assert_eq!(
                Cli::try_parse_from(["nidoo", "--data", "/srv/nidoo"]).unwrap(),
                run("/srv/nidoo")
            );
            assert_eq!(
                Cli::try_parse_from(["nidoo", "run", "--data", "d"]).unwrap(),
                run("d")
            );
            jail.set_env("NIDOO_DATA_DIR", "/data");
            assert_eq!(Cli::try_parse_from(["nidoo"]).unwrap(), run("/data"));
            // The flag beats the environment.
            assert_eq!(
                Cli::try_parse_from(["nidoo", "--data", "x"]).unwrap(),
                run("x")
            );
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
                }
            );
            assert_eq!(
                Cli::try_parse_from(["nidoo", "--data", "/data", "init-bundled"]).unwrap(),
                Cli::InitBundled {
                    data_dir: "/data".into(),
                }
            );
            Ok(())
        });
    }
}
