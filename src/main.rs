use std::path::PathBuf;

use fvp_rs::{Result, extract, pack};
use palc::{Parser, Subcommand};
use path_absolutize::Absolutize;
use tap::Tap;

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    subcommand: Subcommand,
}

#[derive(Subcommand)]
enum Subcommand {
    Unpack {
        input_file: PathBuf,
        output_dir: Option<PathBuf>,
    },
    Pack {
        input_dir: PathBuf,
        output_file: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.subcommand {
        Subcommand::Unpack {
            input_file,
            output_dir,
        } => {
            extract(
                &input_file,
                output_dir.unwrap_or_else(|| input_file.with_extension("")),
            )?;
        }
        Subcommand::Pack {
            input_dir,
            output_file,
        } => {
            pack(
                &input_dir,
                output_file.unwrap_or_else(|| {
                    input_dir
                        .absolutize()
                        .expect("cannot absolutize output path")
                        .as_os_str()
                        .to_owned()
                        .tap_mut(|x| x.push(".bin"))
                        .into()
                }),
            )?;
        }
    }
    Ok(())
}
