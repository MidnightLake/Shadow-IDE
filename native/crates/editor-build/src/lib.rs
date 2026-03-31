pub mod diagnostics;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowProjectConfig {
    pub name: String,
    pub runtime: String,
    pub entry_scene: String,
    pub game_library_name: String,
    pub build: BuildConfig,
}

impl ShadowProjectConfig {
    pub fn example() -> Self {
        Self {
            name: "Empty3D".into(),
            runtime: "cpp23".into(),
            entry_scene: "scenes/Main.shadow".into(),
            game_library_name: "libhello_game.so".into(),
            build: BuildConfig {
                compiler: "clang++".into(),
                standard: "c++23".into(),
                include_dirs: vec!["../../runtime-sdk/include".into(), "src".into()],
                defines: vec!["SHADOW_DEBUG".into(), "SHADOW_EDITOR".into()],
                link_libs: Vec::new(),
            },
        }
    }

    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to read project config {}", path.display()))?;
        toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildConfig {
    pub compiler: String,
    pub standard: String,
    pub include_dirs: Vec<String>,
    pub defines: Vec<String>,
    pub link_libs: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct BuildOrchestrator {
    pub project_root: PathBuf,
    pub config: ShadowProjectConfig,
}

impl BuildOrchestrator {
    pub fn from_project_root(project_root: impl AsRef<Path>) -> Result<Self> {
        let project_root = project_root.as_ref().to_path_buf();
        let config_path = project_root.join(".shadow_project.toml");
        let config = ShadowProjectConfig::from_path(&config_path)?;
        Ok(Self { project_root, config })
    }

    pub fn example(project_root: impl AsRef<Path>) -> Self {
        Self {
            project_root: project_root.as_ref().to_path_buf(),
            config: ShadowProjectConfig::example(),
        }
    }

    pub fn source_root(&self) -> PathBuf {
        self.project_root.join("src")
    }

    pub fn compile_commands_path(&self) -> PathBuf {
        self.project_root.join("compile_commands.json")
    }

    pub fn entry_scene_path(&self) -> PathBuf {
        self.project_root.join(&self.config.entry_scene)
    }

    pub fn reflection_output_path(&self) -> PathBuf {
        self.project_root
            .join(".shadoweditor")
            .join("shadow_reflect.json")
    }

    pub fn runtime_library_path(&self) -> PathBuf {
        self.project_root.join("build").join(&self.config.game_library_name)
    }

    pub fn status_line(&self) -> String {
        let compile_commands = if self.compile_commands_path().exists() {
            "compile_commands ready"
        } else {
            "compile_commands pending"
        };
        let reflection = if self.reflection_output_path().exists() {
            "reflection ready"
        } else {
            "reflection pending"
        };

        format!(
            "{} {} | {} include dirs | {} | {}",
            self.config.build.compiler,
            self.config.build.standard,
            self.config.build.include_dirs.len(),
            compile_commands,
            reflection
        )
    }

    pub fn trigger_build(&self) -> Result<BuildOutput> {
        let start = Instant::now();
        let source_files = self.collect_source_files()?;

        if source_files.is_empty() {
            bail!("no C++ source files found in {}", self.source_root().display());
        }

        let ninja_file = self.project_root.join("build/build.ninja");
        let (success, output) = if ninja_file.exists() {
            let out = Command::new("ninja")
                .current_dir(self.project_root.join("build"))
                .output()
                .context("failed to invoke ninja")?;
            (
                out.status.success(),
                format!(
                    "{}{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                ),
            )
        } else {
            let build_dir = self.project_root.join("build");
            fs::create_dir_all(&build_dir)?;
            let out_lib = build_dir.join(&self.config.game_library_name);

            let mut cmd = Command::new(&self.config.build.compiler);
            cmd.arg(format!("-std={}", self.config.build.standard))
                .arg("-shared")
                .arg("-fPIC")
                .arg("-O0")
                .arg("-g");

            for inc in self.resolved_include_dirs() {
                cmd.arg(format!("-I{}", inc.display()));
            }
            for def in &self.config.build.defines {
                cmd.arg(format!("-D{}", def));
            }
            for file in &source_files {
                cmd.arg(file);
            }
            for lib in &self.config.build.link_libs {
                if lib.starts_with("-l") || lib.starts_with("-L") {
                    cmd.arg(lib);
                } else {
                    cmd.arg(format!("-l{}", lib));
                }
            }
            cmd.arg("-o").arg(&out_lib);
            cmd.current_dir(&self.project_root);

            let out = cmd.output().context("failed to invoke compiler")?;
            (
                out.status.success(),
                format!(
                    "{}{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                ),
            )
        };

        let duration_ms = start.elapsed().as_millis() as u64;
        let log_dir = self.project_root.join(".shadoweditor");
        let _ = fs::create_dir_all(&log_dir);
        let _ = fs::write(log_dir.join("last_build.log"), &output);

        Ok(BuildOutput {
            success,
            output,
            duration_ms,
        })
    }

    pub fn generate_compile_commands(&self) -> Result<()> {
        let source_files = self.collect_source_files()?;
        if source_files.is_empty() {
            bail!("no .cpp/.cc/.cxx files found in {}", self.source_root().display());
        }

        let include_flags: Vec<String> = self
            .resolved_include_dirs()
            .into_iter()
            .map(|path| format!("-I{}", path.display()))
            .collect();
        let define_flags: Vec<String> = self
            .config
            .build
            .defines
            .iter()
            .map(|define| format!("-D{}", define))
            .collect();

        let base_flags: Vec<String> = vec![
            self.config.build.compiler.clone(),
            format!("-std={}", self.config.build.standard),
            "-shared".into(),
            "-fPIC".into(),
            "-O0".into(),
            "-g".into(),
        ]
        .into_iter()
        .chain(include_flags)
        .chain(define_flags)
        .collect();

        let entries: Vec<serde_json::Value> = source_files
            .iter()
            .map(|file| {
                let mut args = base_flags.clone();
                args.push(file.display().to_string());
                serde_json::json!({
                    "directory": self.project_root.display().to_string(),
                    "file": file.display().to_string(),
                    "arguments": args,
                })
            })
            .collect();

        let json = serde_json::to_string_pretty(&entries)?;
        fs::write(self.compile_commands_path(), json)?;
        Ok(())
    }

    pub fn generate_reflection(&self) -> Result<PathBuf> {
        let mut headers = self.collect_header_files()?;
        headers.sort();

        let mut components = Vec::new();
        for header in headers {
            let doc = parse_header(&header)?;
            components.extend(doc.components);
        }

        let output_path = self.reflection_output_path();
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&ReflectionDoc { components })?;
        fs::write(&output_path, json)?;
        Ok(output_path)
    }

    pub fn collect_source_files(&self) -> Result<Vec<PathBuf>> {
        collect_files_with_extensions(&self.source_root(), &["cpp", "cc", "cxx"])
    }

    pub fn collect_header_files(&self) -> Result<Vec<PathBuf>> {
        collect_files_with_extensions(&self.source_root(), &["h", "hpp", "hh", "hxx", "inl", "ixx"])
    }

    fn resolved_include_dirs(&self) -> Vec<PathBuf> {
        self.config
            .build
            .include_dirs
            .iter()
            .map(|inc| {
                if Path::new(inc).is_absolute() {
                    PathBuf::from(inc)
                } else {
                    self.project_root.join(inc)
                }
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct BuildOutput {
    pub success: bool,
    pub output: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
struct ReflectionDoc {
    components: Vec<ComponentDoc>,
}

#[derive(Debug, Clone, Serialize)]
struct ComponentDoc {
    name: String,
    properties: Vec<PropertyDoc>,
}

#[derive(Debug, Clone, Serialize)]
struct PropertyDoc {
    name: String,
    ty: String,
    metadata: String,
}

fn collect_files_with_extensions(root: &Path, extensions: &[&str]) -> Result<Vec<PathBuf>> {
    if !root.exists() {
        bail!("{} not found", root.display());
    }

    let mut files = Vec::new();
    collect_matching_files(root, extensions, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_matching_files(dir: &Path, extensions: &[&str], out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|value| value.to_str()).unwrap_or("");
            if !name.starts_with('.') && name != "build" {
                collect_matching_files(&path, extensions, out)?;
            }
            continue;
        }

        let ext = path.extension().and_then(|value| value.to_str()).unwrap_or("");
        if extensions.iter().any(|candidate| candidate.eq_ignore_ascii_case(ext)) {
            out.push(path);
        }
    }
    Ok(())
}

fn parse_header(path: &Path) -> Result<ReflectionDoc> {
    let source = fs::read_to_string(path)
        .with_context(|| format!("failed to read header {}", path.display()))?;
    let mut components = Vec::new();
    let mut current_component: Option<ComponentDoc> = None;
    let mut waiting_for_component_name = false;
    let mut pending_property: Option<(String, String)> = None;

    for raw_line in source.lines() {
        let line = raw_line.trim();

        if line.contains("SHADOW_COMPONENT()") {
            if let Some(component) = current_component.take() {
                components.push(component);
            }
            waiting_for_component_name = true;
            continue;
        }

        if waiting_for_component_name {
            if let Some(name) = extract_struct_name(line) {
                current_component = Some(ComponentDoc {
                    name,
                    properties: Vec::new(),
                });
                waiting_for_component_name = false;
                continue;
            }
        }

        if let Some((ty, metadata)) = extract_property_macro(line) {
            pending_property = Some((ty, metadata));
            continue;
        }

        if let Some((ty, metadata)) = pending_property.take() {
            if let Some(field_name) = extract_field_name(line) {
                if let Some(component) = current_component.as_mut() {
                    component.properties.push(PropertyDoc {
                        name: field_name,
                        ty,
                        metadata,
                    });
                }
            } else {
                pending_property = Some((ty, metadata));
            }
        }

        if line.starts_with("};") || line == "}" {
            if let Some(component) = current_component.take() {
                components.push(component);
            }
        }
    }

    if let Some(component) = current_component.take() {
        components.push(component);
    }

    Ok(ReflectionDoc { components })
}

fn extract_struct_name(line: &str) -> Option<String> {
    let line = line.trim_start();
    let remainder = line.strip_prefix("struct ")?;
    let name = remainder
        .split(|ch: char| ch == '{' || ch.is_whitespace())
        .find(|segment| !segment.is_empty())?;
    Some(name.to_string())
}

fn extract_property_macro(line: &str) -> Option<(String, String)> {
    let start = line.find("SHADOW_PROPERTY(")?;
    let inner = &line[start + "SHADOW_PROPERTY(".len()..line.rfind(')')?];
    let mut parts = inner.splitn(2, ',');
    let ty = parts.next()?.trim().to_string();
    let metadata = parts
        .next()
        .map(|raw| raw.trim().trim_matches('"').to_string())
        .unwrap_or_default();
    Some((ty, metadata))
}

fn extract_field_name(line: &str) -> Option<String> {
    if line.is_empty() || line.starts_with("//") || line.starts_with("SHADOW_") {
        return None;
    }

    let statement = line.trim_end_matches(';');
    let statement = statement.split('=').next()?.trim();
    let token = statement
        .split_whitespace()
        .last()?
        .trim_start_matches('*')
        .trim_start_matches('&');
    let token = token.split('[').next().unwrap_or(token);

    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}
