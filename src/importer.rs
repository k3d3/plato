extern crate serde_json;
extern crate reqwest;

use std::env;
use std::fs;
use std::io::Read;
use std::path::{self, Path, PathBuf};
use regex::Regex;
use fnv::FnvHashSet;
use getopts::Options;
use titlecase::titlecase;
use helpers::{load_json, save_json};
use metadata::{Info, FileInfo, Metadata, METADATA_FILENAME, IMPORTED_MD_FILENAME};
use document::{Document, file_kind, open, asciify};
use html_entities::decode_html_entities;
use symbolic_path;
use errors::*;

pub fn run() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();

    let mut opts = Options::new();

    opts.optflag("h", "help", "Print this help message.");
    opts.optflag("I", "import", "Import new books.");
    opts.optflag("S", "extract-isbn", "Try to extract identifiers from books.");
    opts.optflag("R", "retreive-metadata", "Try to retreive missing metadata.");
    opts.optflag("s", "strict", "Only use the ISBN when retreiving metadata.");
    opts.optflag("M", "extract-metadata", "Try to extract metadata from the books.");
    opts.optflag("C", "consolidate", "Consolidate an existing database.");
    opts.optflag("N", "rename", "Rename files based on their info.");
    opts.optflag("Z", "initialize", "Initialize a database.");
    opts.optopt("i", "input", "Input file name.", "INPUT_NAME");
    opts.optopt("o", "output", "Output file name.", "OUTPUT_NAME");

    let matches = opts.parse(&args).chain_err(
        || "Failed to parse the command line arguments.",
    )?;

    if matches.opt_present("h") {
        println!("{}", opts.usage("Usage: plato -h|-I|-S|-R[s]|-M|-C|-N|-Z [-i INPUT_NAME] [-o OUTPUT_NAME] LIBRARY_PATH"));
        return Ok(());
    }

    if matches.free.is_empty() {
        return Err(Error::from("Missing required argument: library path."));
    }

    let library_path = Path::new(&matches.free[0]);
    let input_name = matches.opt_str("i").unwrap_or_else(|| METADATA_FILENAME.to_string());
    let output_name = matches.opt_str("o").unwrap_or_else(|| IMPORTED_MD_FILENAME.to_string());

    let input_path = library_path.join(&input_name);
    let output_path = library_path.join(&output_name);


    if matches.opt_present("Z") {
        if input_path.exists() {
            return Err(Error::from(format!("File already exists: {}.", input_path.display())));
        } else {
            save_json::<Metadata, _>(&vec![], input_path)?;
        }
    } else if matches.opt_present("I") {
        let metadata = load_json(input_path)?;
        let metadata = import(library_path, &metadata)?;
        save_json(&metadata, output_path)?;
    } else {
        let mut metadata = load_json(&output_path)?;

        if matches.opt_present("S") {
            extract_isbn(library_path, &mut metadata);
        }

        if matches.opt_present("R") {
            retreive(&mut metadata, matches.opt_present("s"));
        }

        if matches.opt_present("M") {
            extract_metadata(library_path, &mut metadata);
        }

        if matches.opt_present("C") {
            consolidate(&mut metadata);
        }

        if matches.opt_present("N") {
            rename(library_path, &mut metadata);
        }
        
        save_json(&metadata, output_path)?;
    }

    Ok(())
}

pub fn import(dir: &Path, metadata: &Metadata) -> Result<Metadata> {
    let files = find_files(dir, dir)?;
    let known: FnvHashSet<PathBuf> = metadata.iter()
                                             .map(|info| info.file.path.clone())
                                             .collect();
    let mut metadata = Vec::new();

    for file_info in &files {
        if !known.contains(&file_info.path) {
            println!("{}", file_info.path.display());
            let mut info = Info::default();
            info.file = file_info.clone();
            if let Some(p) = info.file.path.parent() {
                let categ = p.to_string_lossy()
                             .replace(symbolic_path::PATH_SEPARATOR, "")
                             .replace(path::MAIN_SEPARATOR, &symbolic_path::PATH_SEPARATOR.to_string());
                if !categ.is_empty() {
                    info.categories = [categ].iter().cloned().collect();
                }
            }
            metadata.push(info);
        }
    }

    Ok(metadata)
}

pub fn extract_isbn(dir: &Path, metadata: &mut Metadata) {
    for info in metadata.iter_mut() {
        if !info.isbn.is_empty() {
            continue;
        }

        let path = dir.join(&info.file.path);

        if let Some(isbn) = open(&path).and_then(|d| d.isbn()) {
            println!("{}", isbn);
            info.isbn = isbn;
        }
    }
}

pub fn extract_metadata(dir: &Path, metadata: &mut Metadata) {
    for info in metadata.iter_mut() {
        if !info.title.is_empty() {
            continue;
        }

        let path = dir.join(&info.file.path);

        if let Some(doc) = open(&path) {
            info.title = doc.title().unwrap_or_default();
            info.author = doc.author().unwrap_or_default();
        }
    }
}

pub fn retreive(metadata: &mut Metadata, strict: bool) {
    for info in metadata.iter_mut() {
        if !info.title.is_empty() {
            continue;
        }

        let terms = if info.isbn.is_empty() && !strict {
            label_from_path(&info.file.path)
        } else {
            info.isbn.clone()
        };

        if terms.is_empty() {
            continue;
        }

        let url = format!("http://lookupbyisbn.com/Search/Book/{}/1", &terms);

        if let Ok(mut resp) = reqwest::get(&url) {
            if resp.status().is_success() {
                let mut content = String::new();
                resp.read_to_string(&mut content).unwrap();
                let re = Regex::new(r"(?xs)/Lookup/Book/.+?>
                                      ([^<]+)<.+?
                                      <u>([^<]+)</u>.+?
                                      <i>([^<]+)</i>.+?
                                      <i>([^<]+)</i>").unwrap();
                if let Some(caps) = re.captures(&content) {
                    info.title = decode_html_entities(&caps[1]).unwrap_or_default();
                    info.author = decode_html_entities(&caps[2]).unwrap_or_default();
                    info.publisher = decode_html_entities(&caps[3]).unwrap_or_default();
                    info.year = decode_html_entities(&caps[4]).unwrap_or_default();
                    println!("{}", info.label());
                }
            } else {
                eprintln!("The request failed: {:?}.", resp.status());
            }
        }
    }
}

pub fn consolidate(metadata: &mut Metadata) {
    for info in metadata.iter_mut() {
        if info.subtitle.is_empty() {
            let colon = info.title.find(':');

            if colon.is_some() {
                let cur_title = info.title.clone();
                let (title, subtitle) = cur_title.split_at(colon.unwrap());
                info.title = title.trim_right().to_string();
                info.subtitle = subtitle[1..].trim_left().to_string();
            }
        }

        if info.language.is_empty() {
            info.title = titlecase(&info.title);
            info.subtitle = titlecase(&info.subtitle);
        }

        info.title = info.title.replace('\'', "’");
        info.subtitle = info.subtitle.replace('\'', "’");
        info.author = info.author.replace('\'', "’");
        if info.year.len() > 4 {
            info.year = info.year[..4].to_string();
        }
        info.series = info.series.replace('\'', "’");
        info.publisher = info.publisher.replace('\'', "’");
    }
}

pub fn rename(dir: &Path, metadata: &mut Metadata) {
    for info in metadata.iter_mut() {
        let new_file_name = file_name_from_info(info);
        if !new_file_name.is_empty() {
            let old_rel_path = info.file.path.clone();
            let new_rel_path = old_rel_path.with_file_name(&new_file_name);
            if old_rel_path != new_rel_path {
                match fs::rename(dir.join(&old_rel_path), dir.join(&new_rel_path)) {
                    err @ Err(_) => println!("Can't rename {} to {}: {:?}.",
                                             old_rel_path.display(),
                                             new_rel_path.display(), err),
                    Ok(_) => info.file.path = new_rel_path,
                }
            }
        }
    }
}

pub fn file_name_from_info(info: &Info) -> String {
    if info.title.is_empty() {
        return "".to_string();
    }
    let mut base = asciify(&info.title);
    if !info.subtitle.is_empty() {
        base = format!("{} - {}", base, asciify(&info.subtitle));
    }
    if !info.volume.is_empty() && info.series.is_empty() {
        base = format!("{} - {}", base, info.volume);
    }
    if !info.number.is_empty() {
        base = format!("{} - {}", base, info.number);
    }
    if !info.author.is_empty() {
        base = format!("{} - {}", base, asciify(&info.author));
    }
    base = format!("{}.{}", base, info.file.kind);
    base.replace("..", ".").replace(" / ", ", ")
}

pub fn label_from_path(path: &Path) -> String {
    path.file_stem().and_then(|p| p.to_str())
        .map(|t| t.replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '\'', " ")).unwrap_or_default()
}

pub fn find_files(root: &Path, dir: &Path) -> Result<Vec<FileInfo>> {
    let mut result = Vec::new();

    for entry in fs::read_dir(dir).chain_err(|| "Can't read directory.")? {
        let entry = entry.chain_err(|| "Can't read directory entry.")?;
        let path = entry.path();

        if path.is_dir() {
            result.extend_from_slice(&find_files(root, path.as_path())?);
        } else {
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }

            let relat = path.strip_prefix(root).unwrap().to_path_buf();
            let kind = file_kind(path).unwrap_or_default();
            let size = entry.metadata().map(|m| m.len()).unwrap_or_default();

            result.push(
                FileInfo {
                    path: relat,
                    kind,
                    size,
                }
            );
        }
    }

    Ok(result)
}
