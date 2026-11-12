// What the site says, read from the same file the website uses (src/data/site.json), built
// into the binary.

use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Deserialize)]
pub struct Site {
    pub site: Meta,
    /// *stars* mark the words that stand out
    pub intro: String,
    pub projects: Vec<Project>,
    pub links: Vec<Link>,
}

#[derive(Deserialize)]
pub struct Meta {
    pub name: String,
    pub domain: String,
}

#[derive(Deserialize)]
pub struct Project {
    pub name: String,
    pub url: String,
    pub desc: String,
}

#[derive(Deserialize)]
pub struct Link {
    pub label: String,
    pub text: String,
    pub url: String,
}

pub fn site() -> &'static Site {
    static SITE: OnceLock<Site> = OnceLock::new();
    SITE.get_or_init(|| {
        serde_json::from_str(include_str!("../../src/data/site.json")).expect("src/data/site.json doesn't parse")
    })
}
