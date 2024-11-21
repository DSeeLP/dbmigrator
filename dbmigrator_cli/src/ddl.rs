use handlebars::Handlebars;
use pgarchive::{Archive, TocEntry};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
pub struct PgDdlRule {
    #[serde(default)]
    pub empty_namespace: bool,
    pub desc_pattern: Option<String>,
    pub tag_pattern: Option<String>,
    pub filename: String,
}

#[derive(Debug, Clone, Serialize)]
struct TemplateData {
    namespace: String,
    desc_parts: Vec<String>,
    tag_parts: Vec<String>,
}

#[derive(Debug)]
struct PgDdlMatcher {
    empty_namespace: bool,
    desc_regex: regex::Regex,
    tag_regex: regex::Regex,
    filename_template: String,
}

impl PgDdlMatcher {
    fn new(handlebars: &mut Handlebars, rule: &PgDdlRule) -> Result<Self, regex::Error> {
        let desc_pattern = rule.desc_pattern.as_deref().unwrap_or(".*");
        let desc_pattern = desc_pattern.replace("{name}", r#"([[:word:]-]+|\"[[:word:]- ]+\")"#);
        let desc_regex = regex::Regex::new(&desc_pattern)?;
        let tag_pattern = rule.tag_pattern.as_deref().unwrap_or(".*");
        let tag_pattern = tag_pattern.replace("{name}", r#"([[:word:]-]+|\"[[:word:]- ]+\")"#);
        let tag_regex = regex::Regex::new(&tag_pattern)?;
        handlebars
            .register_template_string(&rule.filename, &rule.filename)
            .unwrap();
        Ok(PgDdlMatcher {
            empty_namespace: rule.empty_namespace,
            desc_regex: desc_regex,
            tag_regex: tag_regex,
            filename_template: rule.filename.clone(),
        })
    }

    fn matches(
        &self,
        handlebars: &Handlebars,
        entry: &TocEntry,
    ) -> Result<Option<String>, handlebars::RenderError> {
        if self.empty_namespace != entry.namespace.is_empty() {
            return Ok(None);
        }
        if let Some(desc_captures) = self.desc_regex.captures(&entry.desc) {
            if let Some(tag_captures) = self.tag_regex.captures(&entry.tag) {
                let data = TemplateData {
                    namespace: entry.namespace.clone(),
                    desc_parts: desc_captures
                        .iter()
                        .map(|m| m.unwrap().as_str().to_string())
                        .collect(),
                    tag_parts: tag_captures
                        .iter()
                        .map(|m| m.unwrap().as_str().to_string())
                        .collect(),
                };
                return Ok(Some(handlebars.render(&self.filename_template, &data)?));
            }
        }
        Ok(None)
    }
}

#[derive(Debug)]
pub struct PgDdlConfig<'a> {
    handlebars: Handlebars<'a>,
    matchers: Vec<PgDdlMatcher>,
}

impl<'a> PgDdlConfig<'a> {
    pub fn new() -> Self {
        PgDdlConfig {
            handlebars: Handlebars::new(),
            matchers: Vec::new(),
        }
    }

    pub fn set_ruleset_from_str(&mut self, ruleset: &str) -> Result<(), serde_yaml::Error> {
        let ruleset: Vec<PgDdlRule> = serde_yaml::from_str(ruleset)?;
        self.matchers.clear();
        self.push_ruleset(&ruleset);
        Ok(())
    }

    pub fn push_ruleset(&mut self, ruleset: &Vec<PgDdlRule>) {
        for rule in ruleset.iter() {
            match PgDdlMatcher::new(&mut self.handlebars, rule) {
                Ok(matcher) => self.matchers.push(matcher),
                Err(e) => eprintln!("Error compiling regex: {}", e),
            }
        }
    }

    pub fn gen_filename(&self, entry: &TocEntry) -> Option<String> {
        for matcher in self.matchers.iter() {
            match matcher.matches(&self.handlebars, entry) {
                Ok(Some(filename)) => return Some(filename),
                Ok(None) => (),
                Err(e) => {
                    eprintln!("Error rendering template: {}", e);
                }
            }
        }
        None
    }

    pub fn analyze_pgarchive(&self, archive: Archive, flatten_folder: i8) -> HashMap<String, String> {
        let mut sql_files: HashMap<String, String> = HashMap::new();
        for entry in archive.toc_entries {
            let filename = self
                .gen_filename(&entry)
                .unwrap_or("unclassified.sql".to_string());
            let filename = if flatten_folder > 0 {
                filename.replacen("/", "-", flatten_folder as usize)
            } else {
                filename
            };
            let e = sql_files
                .entry(filename.clone())
                .or_insert("-- Auto-generated by dbmigrator. DO NOT EDIT!\n".to_string());
            e.push_str(
                format!(
                    "-- Name: {}; Type: {}; Schema: {}; Owner: {}\n",
                    entry.tag,
                    entry.desc,
                    if entry.namespace.is_empty() {
                        "-"
                    } else {
                        &entry.namespace
                    },
                    entry.owner
                )
                .as_str(),
            );
            if !entry.defn.is_empty() {
                e.push_str(entry.defn.as_str());
            }
            if !entry.drop_stmt.is_empty() {
                e.push_str("-- ");
                e.push_str(entry.drop_stmt.as_str());
            }
            if !entry.copy_stmt.is_empty() {
                e.push_str("-- ");
                e.push_str(entry.copy_stmt.as_str());
            }
            e.push_str("\n");
        }
        sql_files
    }
}
