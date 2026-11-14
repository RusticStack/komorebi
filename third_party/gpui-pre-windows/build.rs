#![allow(clippy::disallowed_methods, reason = "build scripts are exempt")]

fn main() {
    #[cfg(not(all(target_os = "windows", not(debug_assertions))))]
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(target_os = "windows")]
    {
        // Compile HLSL shaders
        #[cfg(not(debug_assertions))]
        compile_shaders();
    }
}

#[cfg(all(target_os = "windows", not(debug_assertions)))]
mod shader_compilation {
    use std::{
        fs,
        io::Write,
        path::{Path, PathBuf},
        process::{self, Command},
    };

    pub fn compile_shaders() {
        let shader_path =
            PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("src/shaders.hlsl");
        let out_dir = std::env::var("OUT_DIR").unwrap();

        println!("cargo:rerun-if-changed={}", shader_path.display());

        // Check if fxc.exe is available
        let fxc_path = find_fxc_compiler();

        // Define all modules
        let modules = [
            "quad",
            "shadow",
            "path_rasterization",
            "path_sprite",
            "underline",
            "monochrome_sprite",
            "subpixel_sprite",
            "polychrome_sprite",
        ];

        let rust_binding_path = format!("{}/shaders_bytes.rs", out_dir);
        if Path::new(&rust_binding_path).exists() {
            fs::remove_file(&rust_binding_path)
                .expect("Failed to remove existing Rust binding file");
        }
        for module in modules {
            compile_shader_for_module(
                module,
                &out_dir,
                &fxc_path,
                shader_path.to_str().unwrap(),
                &rust_binding_path,
            );
        }

        {
            let shader_path = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
                .join("src/color_text_raster.hlsl");
            compile_shader_for_module(
                "emoji_rasterization",
                &out_dir,
                &fxc_path,
                shader_path.to_str().unwrap(),
                &rust_binding_path,
            );
        }
    }

    /// Locate `binary` in the newest installed Windows SDK.
    pub fn find_latest_windows_sdk_binary(
        binary: &str,
    ) -> Result<Option<PathBuf>, Box<dyn std::error::Error>> {
        let key = windows_registry::LOCAL_MACHINE
            .open("SOFTWARE\\WOW6432Node\\Microsoft\\Microsoft SDKs\\Windows\\v10.0")?;

        let install_folder: String = key.get_string("InstallationFolder")?; // "C:\Program Files (x86)\Windows Kits\10\"
        let install_folder_bin = Path::new(&install_folder).join("bin");

        let mut versions: Vec<_> = std::fs::read_dir(&install_folder_bin)?
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();

        versions.sort_by_key(|s| {
            s.split('.')
                .filter_map(|p| p.parse().ok())
                .collect::<Vec<u32>>()
        });

        let arch = match std::env::consts::ARCH {
            "x86_64" => "x64",
            "aarch64" => "arm64",
            _ => Err(format!(
                "Unsupported architecture: {}",
                std::env::consts::ARCH
            ))?,
        };

        if let Some(highest_version) = versions.last() {
            return Ok(Some(
                install_folder_bin
                    .join(highest_version)
                    .join(arch)
                    .join(binary),
            ));
        }

        Ok(None)
    }

    /// You can set the `GPUI_FXC_PATH` environment variable to specify the path to the fxc.exe compiler.
    // KOMOREBI PATCH (fxc): a GPUI_FXC_PATH that names no file is an error
    // (it no longer falls through silently), `where.exe` may print several
    // lines (the first is taken), the newest SDK's fxc.exe must exist, and a
    // miss prints what was tried and how to fix it instead of a bare panic.
    fn find_fxc_compiler() -> String {
        println!("cargo:rerun-if-env-changed=GPUI_FXC_PATH");
        // Check environment variable
        if let Some(path) = std::env::var_os("GPUI_FXC_PATH").filter(|p| !p.is_empty()) {
            if Path::new(&path).is_file() {
                return path.to_string_lossy().into_owned();
            }
            println!(
                "cargo::error=GPUI_FXC_PATH is set to {}, which is not a file. Point it at fxc.exe \
                 from the Windows SDK (for example C:\\Program Files (x86)\\Windows Kits\\10\\bin\\\
                 10.0.26100.0\\x64\\fxc.exe) or unset it to search the SDK. See docs/GPUI-BUILD.md.",
                Path::new(&path).display()
            );
            process::exit(1);
        }

        // Try to find in PATH
        // NOTE: This has to be `where.exe` on Windows, not `where`, it must be ended with `.exe`
        if let Ok(output) = std::process::Command::new("where.exe")
            .arg("fxc.exe")
            .output()
            && output.status.success()
            && let Some(path) = String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::trim)
                .find(|line| Path::new(line).is_file())
        {
            return path.to_string();
        }

        let sdk = find_latest_windows_sdk_binary("fxc.exe");
        if let Ok(Some(path)) = &sdk
            && path.is_file()
        {
            return path.to_string_lossy().into_owned();
        }

        let looked = match sdk {
            Ok(Some(path)) => format!("{} (missing)", path.display()),
            Ok(None) => "no version folder under the Windows 10 SDK's bin folder".to_string(),
            Err(e) => format!("the Windows 10 SDK registry key ({e})"),
        };
        println!(
            "cargo::error=fxc.exe (the HLSL shader compiler GPUI's release build needs) was not \
             found: not in GPUI_FXC_PATH, not on PATH, and not in the newest Windows SDK: {looked}. \
             Install the Windows 10/11 SDK (Visual Studio Installer, \"Windows 11 SDK\") or set \
             GPUI_FXC_PATH to fxc.exe. See docs/GPUI-BUILD.md."
        );
        process::exit(1);
    }

    fn compile_shader_for_module(
        module: &str,
        out_dir: &str,
        fxc_path: &str,
        shader_path: &str,
        rust_binding_path: &str,
    ) {
        // Compile vertex shader
        let output_file = format!("{}/{}_vs.h", out_dir, module);
        let const_name = format!("{}_VERTEX_BYTES", module.to_uppercase());
        compile_shader_impl(
            fxc_path,
            &format!("{module}_vertex"),
            &output_file,
            &const_name,
            shader_path,
            "vs_4_1",
        );
        generate_rust_binding(&const_name, &output_file, rust_binding_path);

        // Compile fragment shader
        let output_file = format!("{}/{}_ps.h", out_dir, module);
        let const_name = format!("{}_FRAGMENT_BYTES", module.to_uppercase());
        compile_shader_impl(
            fxc_path,
            &format!("{module}_fragment"),
            &output_file,
            &const_name,
            shader_path,
            "ps_4_1",
        );
        generate_rust_binding(&const_name, &output_file, rust_binding_path);
    }

    fn compile_shader_impl(
        fxc_path: &str,
        entry_point: &str,
        output_path: &str,
        var_name: &str,
        shader_path: &str,
        target: &str,
    ) {
        let output = Command::new(fxc_path)
            .args([
                "/T",
                target,
                "/E",
                entry_point,
                "/Fh",
                output_path,
                "/Vn",
                var_name,
                "/O3",
                shader_path,
            ])
            .output();

        match output {
            Ok(result) => {
                if result.status.success() {
                    return;
                }
                println!(
                    "cargo::error=Shader compilation failed for {}:\n{}",
                    entry_point,
                    String::from_utf8_lossy(&result.stderr)
                );
                process::exit(1);
            }
            Err(e) => {
                println!("cargo::error=Failed to run fxc for {}: {}", entry_point, e);
                process::exit(1);
            }
        }
    }

    fn generate_rust_binding(const_name: &str, head_file: &str, output_path: &str) {
        let header_content = fs::read_to_string(head_file).expect("Failed to read header file");
        let const_definition = {
            let global_var_start = header_content.find("const BYTE").unwrap();
            let global_var = &header_content[global_var_start..];
            let equal = global_var.find('=').unwrap();
            global_var[equal + 1..].trim()
        };
        let rust_binding = format!(
            "const {}: &[u8] = &{}\n",
            const_name,
            const_definition.replace('{', "[").replace('}', "]")
        );
        let mut options = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(output_path)
            .expect("Failed to open Rust binding file");
        options
            .write_all(rust_binding.as_bytes())
            .expect("Failed to write Rust binding file");
    }
}

#[cfg(all(target_os = "windows", not(debug_assertions)))]
use shader_compilation::compile_shaders;
