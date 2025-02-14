//! Crate to `#include` C++ headers in your Rust code, and generate
//! idiomatic bindings using `cxx`. See [include_cpp] for details.

// Copyright 2020 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//    https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::{
    fs::File,
    io::Write,
    os::unix::prelude::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

use autocxx_engine::preprocess;
use clap::{crate_authors, crate_version, App, Arg, ArgMatches};
use indoc::indoc;
use itertools::Itertools;
use tempfile::TempDir;

static LONG_HELP: &str = indoc! {"
Command line utility to minimize autocxx bug cases.

This is a wrapper for creduce.

Example command-line:
autocxx-reduce -I my-inc-dir -h my-header -d 'generate!(\"MyClass\")' -k -- --n 64
"};

fn main() {
    let matches = App::new("autocxx-reduce")
        .version(crate_version!())
        .author(crate_authors!())
        .about("Reduce a C++ test case")
        .long_about(LONG_HELP)
        .arg(
            Arg::with_name("inc")
                .short("I")
                .long("inc")
                .multiple(true)
                .number_of_values(1)
                .value_name("INCLUDE DIRS")
                .help("include path")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("define")
                .short("D")
                .long("define")
                .multiple(true)
                .number_of_values(1)
                .value_name("DEFINE")
                .help("macro definition")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("header")
                .short("h")
                .long("header")
                .multiple(true)
                .number_of_values(1)
                .required(true)
                .value_name("HEADER")
                .help("header file name")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("directive")
                .short("d")
                .long("directive")
                .multiple(true)
                .number_of_values(1)
                .value_name("DIRECTIVE")
                .help("directives to put within include_cpp!")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("problem")
                .short("p")
                .long("problem")
                .required(true)
                .value_name("PROBLEM")
                .help("problem string we're looking for... may be in logs, or in generated C++, or generated .rs")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("creduce")
                .long("creduce")
                .required(true)
                .value_name("PATH")
                .help("creduce binary location")
                .default_value("/usr/bin/creduce")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("output")
                .short("o")
                .long("output")
                .value_name("OUTPUT")
                .help("where to write minimized output")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("gen-cmd")
                .short("g")
                .long("gen-cmd")
                .value_name("GEN-CMD")
                .help("where to find autocxx-gen")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("keep")
                .short("k")
                .long("keep-dir")
                .help("keep the temporary directory for debugging purposes"),
        )
        .arg(
            Arg::with_name("clang-args")
                .short("c")
                .long("clang-arg")
                .multiple(true)
                .value_name("CLANG_ARG")
                .help("Extra arguments to pass to Clang"),
        )
        .arg(Arg::with_name("creduce-args").last(true).multiple(true))
        .get_matches();
    run(matches).unwrap();
}

fn run(matches: ArgMatches) -> Result<(), std::io::Error> {
    let keep_tmp = matches.is_present("keep");
    let tmp_dir = TempDir::new()?;
    let r = do_run(matches, &tmp_dir);
    if keep_tmp {
        println!(
            "Keeping temp dir created at: {}",
            tmp_dir.into_path().to_str().unwrap()
        );
    }
    r
}

fn do_run(matches: ArgMatches, tmp_dir: &TempDir) -> Result<(), std::io::Error> {
    let incs: Vec<_> = matches
        .values_of("inc")
        .unwrap_or_default()
        .map(PathBuf::from)
        .collect();
    let defs: Vec<_> = matches.values_of("define").unwrap_or_default().collect();
    let headers: Vec<_> = matches.values_of("header").unwrap_or_default().collect();
    let listing_path = tmp_dir.path().join("listing.h");
    create_concatenated_header(&headers, &listing_path)?;
    let concat_path = tmp_dir.path().join("concat.h");
    announce_progress(&format!(
        "Preprocessing {:?} to {:?}",
        listing_path, concat_path
    ));
    preprocess(&listing_path, &concat_path, &incs, &defs)?;
    let rs_path = tmp_dir.path().join("input.rs");
    let directives: Vec<_> = std::iter::once("#include \"concat.h\"\n".to_string())
        .chain(
            matches
                .values_of("directive")
                .unwrap_or_default()
                .map(|s| format!("{}\n", s)),
        )
        .collect();
    create_rs_file(&rs_path, &directives)?;
    let extra_clang_args: Vec<_> = matches
        .values_of("clang-args")
        .unwrap_or_default()
        .collect();
    let default_gen_cmd = std::env::current_exe()?
        .parent()
        .unwrap()
        .join("autocxx-gen")
        .to_str()
        .unwrap()
        .to_string();
    let gen_cmd = matches.value_of("gen-cmd").unwrap_or(&default_gen_cmd);
    run_sample_gen_cmd(gen_cmd, &rs_path, &tmp_dir.path(), &extra_clang_args)?;
    let interestingness_test = tmp_dir.path().join("test.sh");
    create_interestingness_test(
        gen_cmd,
        &interestingness_test,
        matches.value_of("problem").unwrap(),
        &rs_path,
        &extra_clang_args,
    )?;
    run_interestingness_test(&interestingness_test);
    run_creduce(
        matches.value_of("creduce").unwrap(),
        &interestingness_test,
        &concat_path,
        matches.values_of("creduce-args").unwrap_or_default(),
    );
    let output_path = matches.value_of("output");
    match output_path {
        None => print_minimized_case(&concat_path)?,
        Some(output_path) => {
            std::fs::copy(&concat_path, &PathBuf::from(output_path))?;
        }
    };
    Ok(())
}

fn announce_progress(msg: &str) {
    println!("=== {} ===", msg);
}

fn print_minimized_case(concat_path: &Path) -> Result<(), std::io::Error> {
    announce_progress("Completed. Minimized test case:");
    let contents = std::fs::read_to_string(concat_path)?;
    println!("{}", contents);
    Ok(())
}

/// Arguments we always pass to creduce. This pass always seems to cause a crash
/// as far as I can tell, so always exclude it. It may be environment-dependent,
/// of course, but as I'm the primary user of this tool I am ruthlessly removing it.
const CREDUCE_STANDARD_ARGS: &[&str] = &["--remove-pass", "pass_line_markers", "*"];

fn run_creduce<'a>(
    creduce_cmd: &str,
    interestingness_test: &'a Path,
    concat_path: &'a Path,
    creduce_args: impl Iterator<Item = &'a str>,
) {
    announce_progress("creduce");
    let args = std::iter::once(interestingness_test.to_str().unwrap())
        .chain(std::iter::once(concat_path.to_str().unwrap()))
        .chain(CREDUCE_STANDARD_ARGS.iter().copied())
        .chain(creduce_args)
        .collect::<Vec<_>>();
    println!("Command: {} {}", creduce_cmd, args.join(" "));
    Command::new(creduce_cmd)
        .args(args)
        .status()
        .expect("failed to creduce");
}

fn run_sample_gen_cmd(
    gen_cmd: &str,
    rs_file: &Path,
    tmp_dir: &Path,
    extra_clang_args: &[&str],
) -> Result<(), std::io::Error> {
    let args = format_gen_cmd(rs_file, tmp_dir.to_str().unwrap(), extra_clang_args);
    let args = args.collect::<Vec<_>>();
    let args_str = args.join(" ");
    announce_progress(&format!("Running sample gen cmd: {} {}", gen_cmd, args_str));
    Command::new(gen_cmd).args(args).status()?;
    Ok(())
}

fn format_gen_cmd<'a>(
    rs_file: &Path,
    dir: &str,
    extra_clang_args: &'a [&str],
) -> impl Iterator<Item = String> + 'a {
    let args = [
        "-o".to_string(),
        dir.to_string(),
        "-I".to_string(),
        dir.to_string(),
        rs_file.to_str().unwrap().to_string(),
        "--gen-rs-complete".to_string(),
        "--gen-cpp".to_string(),
        "--".to_string(),
    ]
    .to_vec();
    args.into_iter()
        .chain(extra_clang_args.iter().map(|s| s.to_string()))
}

fn create_interestingness_test(
    gen_cmd: &str,
    test_path: &Path,
    problem: &str,
    rs_file: &Path,
    extra_clang_args: &[&str],
) -> Result<(), std::io::Error> {
    announce_progress("Creating interestingness test");
    // Ensure we refer to the input header by relative path
    // because creduce will invoke us in some other directory with
    // a copy thereof.
    let mut args = format_gen_cmd(rs_file, "$(pwd)", extra_clang_args);
    let args = args.join(" ");
    let content = format!(
        indoc! {"
        #!/bin/sh
        ({} {} 2>&1 && cat gen.complete.rs && cat autocxxgen.h) | grep \"{}\"  >/dev/null 2>&1
    "},
        gen_cmd, args, problem
    );
    println!("Interestingness test:\n{}", content);
    {
        let mut file = File::create(test_path)?;
        file.write_all(content.as_bytes())?;
    }

    let mut perms = std::fs::metadata(&test_path)?.permissions();
    perms.set_mode(0o700);
    std::fs::set_permissions(&test_path, perms)?;
    Ok(())
}

fn run_interestingness_test(test_path: &Path) {
    announce_progress("Running interestingness test");
    let status = Command::new(test_path).status().unwrap();
    announce_progress(&format!(
        "Have run interestingness test - result is {:?}",
        status.code()
    ));
}

fn create_rs_file(rs_path: &Path, directives: &[String]) -> Result<(), std::io::Error> {
    announce_progress("Creating Rust input file");
    let mut file = File::create(rs_path)?;
    file.write_all("use autocxx::include_cpp;\ninclude_cpp! (\n".as_bytes())?;
    for directive in directives {
        file.write_all(directive.as_bytes())?;
    }
    file.write_all(");\n".as_bytes())?;
    Ok(())
}

fn create_concatenated_header(headers: &[&str], listing_path: &Path) -> Result<(), std::io::Error> {
    announce_progress("Creating preprocessed header");
    let mut file = File::create(listing_path)?;
    for header in headers {
        file.write_all(format!("#include \"{}\"\n", header).as_bytes())?;
    }
    Ok(())
}
