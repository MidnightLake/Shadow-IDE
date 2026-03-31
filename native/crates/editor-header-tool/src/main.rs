use anyhow::{Result, bail};
use serde::Serialize;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
struct ReflectionDoc {
    components: Vec<ComponentDoc>,
}

#[derive(Debug, Serialize)]
struct ComponentDoc {
    name: String,
    properties: Vec<PropertyDoc>,
}

#[derive(Debug, Serialize)]
struct PropertyDoc {
    name: String,
    ty: String,
    metadata: String,
}

fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let input = match args.next() {
        Some(path) => path,
        None => {
            bail!("usage: editor-header-tool <header-file-or-src-dir> [output-json]");
        }
    };
    let output = args.next();

    let input_path = Path::new(&input);

    let reflection = if input_path.is_dir() {
        let mut headers: Vec<PathBuf> = Vec::new();
        collect_headers(input_path, &mut headers);
        headers.sort();

        let mut all_components: Vec<ComponentDoc> = Vec::new();
        for header in &headers {
            let doc = parse_header(header)?;
            all_components.extend(doc.components);
        }
        ReflectionDoc { components: all_components }
    } else {
        parse_header(input_path)?
    };

    let json = serde_json::to_string_pretty(&reflection)?;

    if let Some(output) = output {
        fs::write(output, json)?;
    } else {
        println!("{json}");
    }

    Ok(())
}

fn collect_headers(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_headers(&path, out);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if matches!(ext, "h" | "hpp") {
                out.push(path);
            }
        }
    }
}

fn parse_header(path: &Path) -> Result<ReflectionDoc> {
    let source = fs::read_to_string(path)?;
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
