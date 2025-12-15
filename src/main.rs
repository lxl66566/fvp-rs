use std::{
    fs::{self, File},
    io::{BufReader, BufWriter},
    path::{Path, PathBuf},
};

use fvp_rs::{FvpBuilder, FvpReader, Result};
use palc::{Parser, Subcommand};
use path_absolutize::Absolutize;
use tap::Tap;

fn extract(input_file: impl AsRef<Path>, output_dir: impl AsRef<Path>) -> Result<()> {
    let file = File::open(&input_file)?;
    let reader = FvpReader::new(BufReader::new(file))?;

    fs::create_dir_all(&output_dir)?;

    let (entries, mut raw_reader) = reader.into_parts();

    println!("Extracting {} files...", entries.len());

    for entry in &entries {
        let target_path = output_dir.as_ref().join(&entry.name);
        println!("Saving to {:?}...", target_path);

        let mut target_file = File::create(target_path)?;

        entry.extract_from(&mut raw_reader, &mut target_file)?;
    }

    Ok(())
}

fn pack(input_dir: impl AsRef<Path>, output_file: impl AsRef<Path>) -> Result<()> {
    let mut paths: Vec<_> = fs::read_dir(input_dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();

    paths.sort();

    let file_names: Vec<String> = paths
        .iter()
        .filter_map(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .collect();

    let file_names_ref: Vec<&str> = file_names.iter().map(|s| s.as_str()).collect();

    let file = File::create(output_file)?;
    let mut builder = FvpBuilder::new(BufWriter::new(file), &file_names_ref)?;

    for path in &paths {
        println!("Writing {:?}...", path);
        let mut input = File::open(path)?;
        builder.write_file(&mut input)?;
    }

    builder.finish()?;
    println!("Done.");

    Ok(())
}

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

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn test_extract() {
        let temp_dir = tempdir().unwrap();
        let input_file = temp_dir.path().join("input.bin");
        fs::write(&input_file, include_bytes!("../test_assets/test.bin")).unwrap();
        let output_dir = temp_dir.path().join("output");
        extract(&input_file, &output_dir).unwrap();

        assert_eq!(fs::read_to_string(output_dir.join("1.txt")).unwrap(), "111");
        assert_eq!(fs::read_to_string(output_dir.join("2.txt")).unwrap(), "222");
    }

    #[test]
    fn test_pack() {
        let temp_dir = tempdir().unwrap();
        let input_dir = temp_dir.path().join("input");
        fs::create_dir(&input_dir).unwrap();
        fs::write(input_dir.join("1.txt"), "111").unwrap();
        fs::write(input_dir.join("2.txt"), "222").unwrap();

        let output_file = temp_dir.path().join("output.bin");
        pack(&input_dir, &output_file).unwrap();

        assert_eq!(
            fs::read(output_file).unwrap(),
            include_bytes!("../test_assets/test.bin")
        );
    }
}
