//! `oxideav` command-line frontend.

use clap::{Parser, Subcommand};
use oxideav::container::ReadSeek;
use oxideav::core::Error;
use oxideav::Registries;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "oxideav",
    version,
    about = "Pure-Rust media transcoding framework",
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List compiled-in codecs and containers.
    List,
    /// Probe a media file and print stream information.
    Probe {
        /// Path to the input media file.
        input: PathBuf,
    },
    /// Remux an input file to a new container (no re-encoding).
    ///
    /// Only stream copy is supported for now; both sides must use the same codec.
    Remux {
        input: PathBuf,
        output: PathBuf,
        /// Override the output container format. Defaults to file extension.
        #[arg(long)]
        format: Option<String>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let registries = Registries::with_all_features();

    let result = match cli.command {
        Command::List => cmd_list(&registries),
        Command::Probe { input } => cmd_probe(&registries, &input),
        Command::Remux {
            input,
            output,
            format,
        } => cmd_remux(&registries, &input, &output, format.as_deref()),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("oxideav: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_list(reg: &Registries) -> oxideav::core::Result<()> {
    let mut containers: Vec<_> = reg.containers.demuxer_names().collect();
    containers.sort_unstable();
    println!("Containers (demux):");
    for c in &containers {
        println!("  {c}");
    }

    let mut muxers: Vec<_> = reg.containers.muxer_names().collect();
    muxers.sort_unstable();
    println!("Containers (mux):");
    for c in &muxers {
        println!("  {c}");
    }

    let mut dec: Vec<_> = reg
        .codecs
        .decoder_ids()
        .map(|c| c.as_str().to_owned())
        .collect();
    dec.sort_unstable();
    println!("Decoders:");
    for c in &dec {
        println!("  {c}");
    }

    let mut enc: Vec<_> = reg
        .codecs
        .encoder_ids()
        .map(|c| c.as_str().to_owned())
        .collect();
    enc.sort_unstable();
    println!("Encoders:");
    for c in &enc {
        println!("  {c}");
    }

    Ok(())
}

fn cmd_probe(reg: &Registries, input: &Path) -> oxideav::core::Result<()> {
    let format = format_for_path(reg, input)?;
    let file: Box<dyn ReadSeek> = Box::new(File::open(input)?);
    let demuxer = reg.containers.open_demuxer(&format, file)?;
    println!(
        "Input: {}\nFormat: {}",
        input.display(),
        demuxer.format_name()
    );
    for s in demuxer.streams() {
        let p = &s.params;
        print!(
            "  Stream #{} [{:?}]  codec={}  time_base={}",
            s.index, p.media_type, p.codec_id, s.time_base.as_rational()
        );
        if let (Some(ch), Some(sr)) = (p.channels, p.sample_rate) {
            print!("  audio {}ch @ {} Hz", ch, sr);
            if let Some(fmt) = p.sample_format {
                print!("  [{:?}]", fmt);
            }
        }
        if let (Some(w), Some(h)) = (p.width, p.height) {
            print!("  video {}x{}", w, h);
        }
        if let Some(br) = p.bit_rate {
            print!("  {} bps", br);
        }
        if let Some(d) = s.duration {
            print!("  duration={} ticks ({:.3}s)", d, s.time_base.seconds_of(d));
        }
        println!();
    }
    Ok(())
}

fn cmd_remux(
    reg: &Registries,
    input: &Path,
    output: &Path,
    format_override: Option<&str>,
) -> oxideav::core::Result<()> {
    let in_format = format_for_path(reg, input)?;
    let out_format = match format_override {
        Some(f) => f.to_owned(),
        None => format_for_path(reg, output)?,
    };

    let fin: Box<dyn ReadSeek> = Box::new(File::open(input)?);
    let mut demuxer = reg.containers.open_demuxer(&in_format, fin)?;

    let fout: Box<dyn oxideav::container::WriteSeek> =
        Box::new(File::create(output)?);
    let mut muxer = reg.containers.open_muxer(&out_format, fout, demuxer.streams())?;

    let n = oxideav::pipeline::remux(&mut *demuxer, &mut *muxer)?;
    println!(
        "Remuxed {} packet(s) from {} ({}) → {} ({})",
        n,
        input.display(),
        in_format,
        output.display(),
        out_format,
    );
    Ok(())
}

fn format_for_path(reg: &Registries, path: &Path) -> oxideav::core::Result<String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .ok_or_else(|| Error::FormatNotFound(format!("no extension on {}", path.display())))?;
    reg.containers
        .container_for_extension(ext)
        .map(|s| s.to_owned())
        .ok_or_else(|| Error::FormatNotFound(format!("no container registered for .{ext}")))
}
