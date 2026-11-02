//! Reading the JetBrains XML files that list recent projects.

use std::{fs, path::Path};

use anyhow::{Context, Result};
use roxmltree::{Document, Node};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawEntry {
    pub key: String,
    pub open_timestamp: Option<i64>,
    pub activation_timestamp: Option<i64>,
    pub build: Option<String>,
    pub production_code: Option<String>,
}

pub fn parse_recent_projects(path: &Path) -> Result<Vec<RawEntry>> {
    parse_component(path, "RecentProjectsManager")
}

pub fn parse_recent_directories(path: &Path) -> Result<Vec<RawEntry>> {
    parse_recent_paths_component(path, "RecentDirectoryProjectsManager")
}

pub fn parse_recent_solutions(path: &Path) -> Result<Vec<RawEntry>> {
    parse_component(path, "RiderRecentProjectsManager")
}

pub fn parse_trusted_paths(path: &Path) -> Result<Vec<RawEntry>> {
    let text = read(path)?;
    parse_trusted_paths_str(&text)
}

fn parse_component(path: &Path, component_name: &str) -> Result<Vec<RawEntry>> {
    let text = read(path)?;
    parse_component_str(&text, component_name)
}

fn parse_recent_paths_component(path: &Path, component_name: &str) -> Result<Vec<RawEntry>> {
    let text = read(path)?;
    parse_recent_paths_component_str(&text, component_name)
}

fn read(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("read {}", path.display()))
}

#[cfg(test)]
pub fn parse_recent_projects_str(xml: &str) -> Result<Vec<RawEntry>> {
    parse_component_str(xml, "RecentProjectsManager")
}

#[cfg(test)]
pub fn parse_recent_directories_str(xml: &str) -> Result<Vec<RawEntry>> {
    parse_recent_paths_component_str(xml, "RecentDirectoryProjectsManager")
}

#[cfg(test)]
pub fn parse_recent_solutions_str(xml: &str) -> Result<Vec<RawEntry>> {
    parse_component_str(xml, "RiderRecentProjectsManager")
}

pub fn parse_trusted_paths_str(xml: &str) -> Result<Vec<RawEntry>> {
    let doc = Document::parse(xml).context("parse trusted-paths.xml")?;
    let mut entries = Vec::new();
    for option in doc
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "option")
    {
        if option.attribute("name") != Some("TRUSTED_PROJECT_PATHS") {
            continue;
        }
        for entry in option
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == "entry")
        {
            if let Some(key) = entry.attribute("key") {
                entries.push(RawEntry {
                    key: key.to_string(),
                    ..RawEntry::default()
                });
            }
        }
    }
    Ok(entries)
}

fn parse_component_str(xml: &str, component_name: &str) -> Result<Vec<RawEntry>> {
    let doc = Document::parse(xml).with_context(|| format!("parse {component_name}"))?;
    let Some(component) = component(&doc, component_name) else {
        return Ok(Vec::new());
    };

    let mut entries = additional_entries(component);
    if entries.is_empty() {
        entries = recent_paths(component);
    }
    Ok(entries)
}

fn parse_recent_paths_component_str(xml: &str, component_name: &str) -> Result<Vec<RawEntry>> {
    let doc = Document::parse(xml).with_context(|| format!("parse {component_name}"))?;
    let Some(component) = component(&doc, component_name) else {
        return Ok(Vec::new());
    };
    Ok(recent_paths(component))
}

fn component<'a, 'input>(doc: &'a Document<'input>, name: &str) -> Option<Node<'a, 'input>> {
    doc.descendants().find(|node| {
        node.is_element()
            && node.tag_name().name() == "component"
            && node.attribute("name") == Some(name)
    })
}

fn additional_entries(component: Node<'_, '_>) -> Vec<RawEntry> {
    let Some(additional_info) = component.descendants().find(|node| {
        node.is_element()
            && node.tag_name().name() == "option"
            && node.attribute("name") == Some("additionalInfo")
    }) else {
        return Vec::new();
    };

    let mut entries = Vec::new();
    for entry in additional_info
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "entry")
    {
        let Some(key) = entry.attribute("key") else {
            continue;
        };

        let mut raw = RawEntry {
            key: key.to_string(),
            ..RawEntry::default()
        };

        for option in entry
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == "option")
        {
            let Some(name) = option.attribute("name") else {
                continue;
            };
            let Some(value) = option.attribute("value") else {
                continue;
            };
            match name {
                "projectOpenTimestamp" => raw.open_timestamp = value.parse().ok(),
                "activationTimestamp" => raw.activation_timestamp = value.parse().ok(),
                "build" => raw.build = Some(value.to_string()),
                "productionCode" => raw.production_code = Some(value.to_string()),
                _ => {}
            }
        }

        entries.push(raw);
    }
    entries
}

fn recent_paths(component: Node<'_, '_>) -> Vec<RawEntry> {
    let Some(recent_paths) = component.descendants().find(|node| {
        node.is_element()
            && node.tag_name().name() == "option"
            && node.attribute("name") == Some("recentPaths")
    }) else {
        return Vec::new();
    };

    recent_paths
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "option")
        .filter_map(|node| node.attribute("value"))
        .map(|value| RawEntry {
            key: value.to_string(),
            ..RawEntry::default()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECENT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<application>
  <component name="RecentProjectsManager">
    <option name="additionalInfo">
      <map>
        <entry key="$USER_HOME$/Code/alpha">
          <value>
            <RecentProjectMetaInfo>
              <option name="activationTimestamp" value="1775186315981" />
              <option name="build" value="IU-261.22158.277" />
              <option name="productionCode" value="IU" />
              <option name="projectOpenTimestamp" value="1775184258349" />
            </RecentProjectMetaInfo>
          </value>
        </entry>
        <entry key="$USER_HOME$/Code/beta" />
      </map>
    </option>
  </component>
</application>"#;

    #[test]
    fn reads_additional_info() {
        let entries = parse_recent_projects_str(RECENT).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].key, "$USER_HOME$/Code/alpha");
        assert_eq!(entries[0].open_timestamp, Some(1_775_184_258_349));
        assert_eq!(entries[0].activation_timestamp, Some(1_775_186_315_981));
        assert_eq!(entries[0].build.as_deref(), Some("IU-261.22158.277"));
        assert_eq!(entries[0].production_code.as_deref(), Some("IU"));
        assert_eq!(entries[1].key, "$USER_HOME$/Code/beta");
    }

    #[test]
    fn falls_back_to_recent_paths() {
        let xml = r#"<application><component name="RecentProjectsManager">
          <option name="recentPaths"><list>
            <option value="$USER_HOME$/Code/a" />
            <option value="$USER_HOME$/Code/b" />
          </list></option>
        </component></application>"#;
        let entries = parse_recent_projects_str(xml).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].key, "$USER_HOME$/Code/a");
    }

    #[test]
    fn reads_recent_directories() {
        let xml = r#"<application><component name="RecentDirectoryProjectsManager">
          <option name="recentPaths"><list>
            <option value="$USER_HOME$/Code/directory" />
          </list></option>
        </component></application>"#;
        let entries = parse_recent_directories_str(xml).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, "$USER_HOME$/Code/directory");
    }

    #[test]
    fn reads_rider_solutions() {
        let xml = r#"<application><component name="RiderRecentProjectsManager">
          <option name="additionalInfo"><map>
            <entry key="$USER_HOME$/Code/Solution"><value><RecentProjectMetaInfo>
              <option name="projectOpenTimestamp" value="42" />
            </RecentProjectMetaInfo></value></entry>
          </map></option>
        </component></application>"#;
        let entries = parse_recent_solutions_str(xml).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].open_timestamp, Some(42));
    }

    #[test]
    fn reads_fleet_trusted_paths() {
        let xml = r#"<application><component name="TrustedPaths">
          <option name="TRUSTED_PROJECT_PATHS"><map>
            <entry key="$USER_HOME$/Code/fleet" />
          </map></option>
        </component></application>"#;
        let entries = parse_trusted_paths_str(xml).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, "$USER_HOME$/Code/fleet");
    }
}
