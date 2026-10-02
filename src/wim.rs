//! Read the image list from the XML metadata of a WIM/ESD file.

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::image::{Entry, Image, u64_le};

const WIM_MAGIC: &[u8] = b"MSWIM\0\0\0";
const HEADER_SIZE: u64 = 208;
const MAX_XML_SIZE: u64 = 16 * 1024 * 1024;

#[derive(Debug, Default, Serialize)]
pub struct WimImage {
    pub index: u32,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installation_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip)]
    pub build: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
}

/// Read the XML resource of a WIM file and return the contained images.
pub fn read_images(image: &Image, file: &Entry) -> Result<Vec<WimImage>> {
    if file.size < HEADER_SIZE {
        bail!("'{}' is too small for a WIM file", file.name);
    }
    let header = file.read(image, 0, HEADER_SIZE)?;
    if &header[0..8] != WIM_MAGIC {
        bail!("'{}' is not a WIM file", file.name);
    }

    // resource header of the XML data: 7 byte size, 1 byte flags, 8 byte offset
    let reshdr = &header[72..96];
    let size = u64_le(reshdr, 0) & 0x00ff_ffff_ffff_ffff;
    let offset = u64_le(reshdr, 8);
    if size == 0 {
        bail!("'{}' has no XML data", file.name);
    }
    if size > MAX_XML_SIZE {
        bail!("XML data of '{}' is too large ({size} bytes)", file.name);
    }
    let data = file.read(image, offset, size)?;

    let units: Vec<u16> = data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let xml = String::from_utf16_lossy(&units);
    parse_xml(xml.trim_start_matches('\u{feff}'))
}

fn parse_xml(xml: &str) -> Result<Vec<WimImage>> {
    let doc = roxmltree::Document::parse(xml).context("unable to parse WIM XML data")?;

    let mut res = Vec::new();
    for node in doc.root_element().children().filter(|n| n.has_tag_name("IMAGE")) {
        let Some(index) = node.attribute("INDEX").and_then(|i| i.parse().ok()) else {
            continue;
        };
        let windows = child(node, "WINDOWS");
        let version = windows.and_then(|w| child(w, "VERSION"));
        let number = |name| version.and_then(|v| text(v, name)).and_then(|t| t.parse::<u32>().ok());

        let mut img = WimImage {
            index,
            name: text(node, "NAME").unwrap_or_default(),
            description: text(node, "DESCRIPTION"),
            display_name: text(node, "DISPLAYNAME"),
            edition: windows.and_then(|w| text(w, "EDITIONID")),
            installation_type: windows.and_then(|w| text(w, "INSTALLATIONTYPE")),
            arch: windows.and_then(|w| text(w, "ARCH")).map(|a| windows_arch(&a).unwrap_or(&a).to_string()),
            build: number("BUILD"),
            ..Default::default()
        };
        if let (Some(major), Some(minor), Some(build)) = (number("MAJOR"), number("MINOR"), img.build) {
            img.version = Some(match number("SPBUILD") {
                Some(sp) => format!("{major}.{minor}.{build}.{sp}"),
                None => format!("{major}.{minor}.{build}"),
            });
        }
        if let Some(languages) = windows.and_then(|w| child(w, "LANGUAGES")) {
            img.languages = languages
                .children()
                .filter(|n| n.has_tag_name("LANGUAGE"))
                .filter_map(|n| n.text().map(|t| t.trim().to_string()))
                .filter(|t| !t.is_empty())
                .collect();
        }
        res.push(img);
    }
    res.sort_by_key(|img| img.index);

    Ok(res)
}

fn child<'a, 'i>(node: roxmltree::Node<'a, 'i>, name: &str) -> Option<roxmltree::Node<'a, 'i>> {
    node.children().find(|n| n.has_tag_name(name))
}

fn text(node: roxmltree::Node, name: &str) -> Option<String> {
    child(node, name).and_then(|n| n.text()).map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

/// PROCESSOR_ARCHITECTURE values
fn windows_arch(arch: &str) -> Option<&'static str> {
    Some(match arch {
        "0" => "i686",
        "5" => "arm",
        "6" => "ia64",
        "9" => "x86_64",
        "12" => "aarch64",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse() {
        let xml = r#"<WIM><TOTALBYTES>1</TOTALBYTES>
            <IMAGE INDEX="2"><NAME>Windows Server 2022 SERVERSTANDARD</NAME>
              <WINDOWS><ARCH>9</ARCH><EDITIONID>ServerStandard</EDITIONID>
                <INSTALLATIONTYPE>Server</INSTALLATIONTYPE>
                <LANGUAGES><LANGUAGE>en-US</LANGUAGE><DEFAULT>en-US</DEFAULT></LANGUAGES>
                <VERSION><MAJOR>10</MAJOR><MINOR>0</MINOR><BUILD>20348</BUILD><SPBUILD>587</SPBUILD></VERSION>
              </WINDOWS></IMAGE>
            <IMAGE INDEX="1"><NAME>Windows Server 2022 SERVERSTANDARDCORE</NAME>
              <WINDOWS><ARCH>12</ARCH><INSTALLATIONTYPE>Server Core</INSTALLATIONTYPE></WINDOWS></IMAGE>
            </WIM>"#;
        let images = parse_xml(xml).unwrap();
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].index, 1);
        assert_eq!(images[0].arch.as_deref(), Some("aarch64"));
        assert_eq!(images[1].edition.as_deref(), Some("ServerStandard"));
        assert_eq!(images[1].version.as_deref(), Some("10.0.20348.587"));
        assert_eq!(images[1].build, Some(20348));
        assert_eq!(images[1].languages, vec!["en-US"]);
    }
}
