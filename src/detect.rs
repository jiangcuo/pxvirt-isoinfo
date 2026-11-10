//! Apply the detection rules to an opened image.

use std::collections::HashMap;

use anyhow::Result;
use serde::Serialize;

use crate::image::{FileSystem, Image};
use crate::rules::{ContentRule, LinuxRule, Match, Rules};
use crate::wim::{self, WimImage};

/// Files larger than this are never matched against regular expressions.
const MAX_CONTENT_SIZE: u64 = 1024 * 1024;

#[derive(Debug, Default, Serialize)]
pub struct Info {
    /// windows, linux or unknown
    #[serde(rename = "type")]
    pub kind: String,
    pub filesystem: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    /// autoinstall type
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ostype: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<WimImage>,
}

pub struct Detector<'a> {
    image: &'a Image,
    fs: &'a dyn FileSystem,
    rules: &'a Rules,
    /// content of small files, None if missing or too large
    cache: HashMap<String, Option<Vec<u8>>>,
}

impl<'a> Detector<'a> {
    pub fn new(image: &'a Image, fs: &'a dyn FileSystem, rules: &'a Rules) -> Self {
        Self { image, fs, rules, cache: HashMap::new() }
    }

    pub fn detect(&mut self, filesystem: &str) -> Result<Info> {
        let mut info = Info {
            kind: "unknown".to_string(),
            filesystem: filesystem.to_string(),
            label: self.fs.label().to_string(),
            ..Default::default()
        };

        if self.detect_windows(&mut info)? {
            return Ok(info);
        }

        let rules = self.rules;
        for rule in &rules.linux {
            if self.matches(&rule.matches)? {
                self.apply_linux(rule, &mut info)?;
                break;
            }
        }

        Ok(info)
    }

    fn detect_windows(&mut self, info: &mut Info) -> Result<bool> {
        let rules = &self.rules.windows;
        let mut found = None;
        for path in &rules.images {
            if let Some(entry) = self.fs.lookup(self.image, path)?
                && !entry.is_dir
            {
                found = Some(entry);
                break;
            }
        }
        let Some(entry) = found else {
            return Ok(false);
        };

        let images = wim::read_images(self.image, &entry)?;

        info.kind = "windows".to_string();
        info.id = Some("windows".to_string());
        info.installer = rules.installer.clone();
        if let Some(first) = images.first() {
            info.arch = first.arch.clone();
            info.version = first.version.clone();
            let installation_type = first.installation_type.as_deref().unwrap_or("");
            let build = first.build.unwrap_or(0);
            if let Some(version) = rules.versions.iter().find(|v| v.matches(installation_type, build)) {
                info.name = Some(version.name.clone());
                info.ostype = version.ostype.clone();
            }
        }
        info.images = images;

        Ok(true)
    }

    fn apply_linux(&mut self, rule: &LinuxRule, info: &mut Info) -> Result<()> {
        info.kind = "linux".to_string();
        info.id = Some(rule.id.clone());
        info.installer = rule.installer.clone();
        info.ostype = rule.ostype.clone();

        let mut name = None;
        for content in &rule.info {
            let Some(text) = self.text(&content.file)? else {
                continue;
            };
            let Some(caps) = content.regex.0.captures(&text) else {
                continue;
            };
            let group = |n| caps.name(n).map(|m| m.as_str().trim().to_string()).filter(|s| !s.is_empty());
            name = name.or_else(|| group("name"));
            info.version = info.version.take().or_else(|| group("version"));
            info.arch = info.arch.take().or_else(|| group("arch"));
        }
        info.name = Some(name.unwrap_or_else(|| rule.name.clone()));
        info.arch = info.arch.take().map(|arch| self.rules.arch(&arch));

        Ok(())
    }

    fn matches(&mut self, m: &Match) -> Result<bool> {
        if let Some(label) = &m.label
            && !label.0.is_match(self.fs.label())
        {
            return Ok(false);
        }
        for path in &m.files_all {
            if self.fs.lookup(self.image, path)?.is_none() {
                return Ok(false);
            }
        }
        if !m.files_any.is_empty() {
            let mut any = false;
            for path in &m.files_any {
                if self.fs.lookup(self.image, path)?.is_some() {
                    any = true;
                    break;
                }
            }
            if !any {
                return Ok(false);
            }
        }
        for content in &m.content {
            if !self.content_matches(content)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn content_matches(&mut self, content: &ContentRule) -> Result<bool> {
        Ok(self.text(&content.file)?.is_some_and(|text| content.regex.0.is_match(&text)))
    }

    /// Content of a small file as text, `@label` is the volume label.
    fn text(&mut self, path: &str) -> Result<Option<String>> {
        if path == "@label" {
            return Ok(Some(self.fs.label().to_string()));
        }
        if !self.cache.contains_key(path) {
            let data = match self.fs.lookup(self.image, path)? {
                Some(entry) if !entry.is_dir && entry.size <= MAX_CONTENT_SIZE => {
                    Some(entry.read_all(self.image, MAX_CONTENT_SIZE)?)
                }
                _ => None,
            };
            self.cache.insert(path.to_string(), data);
        }
        Ok(self.cache[path].as_ref().map(|data| String::from_utf8_lossy(data).into_owned()))
    }
}
