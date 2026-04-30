//! Building applications linker

use std::fs::{read_dir, File};
use std::io::{Result, Write};
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=../user/src/");
    println!("cargo:rerun-if-changed={}", TARGET_PATH);
    insert_app_data().unwrap();
}

static TARGET_PATH: &str = "../user/build/elf/";

/// get app data and build linker
fn insert_app_data() -> Result<()> {
    let mut f = File::create("src/link_app.S").unwrap();
    let target_path = Path::new(TARGET_PATH);
    let mut apps: Vec<_> = if target_path.exists() {
        read_dir(target_path)?
            .into_iter()
            .map(|dir_entry| {
                let mut name_with_ext = dir_entry.unwrap().file_name().into_string().unwrap();
                name_with_ext.drain(name_with_ext.find('.').unwrap()..name_with_ext.len());
                name_with_ext
            })
            .collect()
    } else {
        println!(
            "cargo:warning=user app directory '{}' not found; building kernel without bundled apps",
            TARGET_PATH
        );
        Vec::new()
    };
    apps.sort();

    writeln!(
        f,
        r#"
    .align 3
    .section .data
    .global _num_app
_num_app:
    .quad {}"#,
        apps.len()
    )?;

    if apps.is_empty() {
        writeln!(f, r#"    .quad app_0_end"#)?;
        writeln!(
            f,
            r#"
    .global _app_names
_app_names:

    .section .data
    .global app_0_end
    .align 3
app_0_end:"#
        )?;
        return Ok(());
    }

    for i in 0..apps.len() {
        writeln!(f, r#"    .quad app_{}_start"#, i)?;
    }
    writeln!(f, r#"    .quad app_{}_end"#, apps.len() - 1)?;

    writeln!(
        f,
        r#"
    .global _app_names
_app_names:"#
    )?;
    for app in apps.iter() {
        writeln!(f, r#"    .string "{}""#, app)?;
    }

    for (idx, app) in apps.iter().enumerate() {
        println!("app_{}: {}", idx, app);
        writeln!(
            f,
            r#"
    .section .data
    .global app_{0}_start
    .global app_{0}_end
    .align 3
app_{0}_start:
    .incbin "{2}{1}.elf"
app_{0}_end:"#,
            idx, app, TARGET_PATH
        )?;
    }
    Ok(())
}
