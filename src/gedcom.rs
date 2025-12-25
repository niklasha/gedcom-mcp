use serde::{Deserialize, Serialize};
use serde_json;
use std::io::ErrorKind;
use std::num::ParseIntError;
use std::{collections::HashMap, fs, path::Path};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub date: Option<String>,
    pub place: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Individual {
    pub id: String,
    pub name: Option<String>,
    pub birth: Option<Event>,
    pub death: Option<Event>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Family {
    pub id: String,
    pub husband: Option<String>,
    pub wife: Option<String>,
    pub children: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GedcomData {
    pub individuals: Vec<Individual>,
    pub families: Vec<Family>,
}

#[derive(Debug, Clone)]
pub struct GedcomStore {
    individuals: HashMap<String, Individual>,
    families: HashMap<String, Family>,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("individual with id {0} already exists")]
    DuplicateIndividual(String),
    #[error("family with id {0} already exists")]
    DuplicateFamily(String),
    #[error("failed to persist GEDCOM data: {0}")]
    Persist(#[from] std::io::Error),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid line level at line {line}: {source}")]
    InvalidLevel { line: usize, source: ParseIntError },
    #[error("missing tag at line {line}")]
    MissingTag { line: usize },
    #[error("individual at line {line} is missing an ID")]
    MissingIndividualId { line: usize },
    #[error("family at line {line} is missing an ID")]
    MissingFamilyId { line: usize },
    #[error("orphaned tag {tag} at line {line}")]
    OrphanTag { line: usize, tag: String },
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Parse(#[from] ParseError),
    #[error("failed to deserialize snapshot: {0}")]
    Deserialize(#[from] serde_json::Error),
}

pub fn parse_gedcom(input: &str) -> Result<GedcomData, ParseError> {
    let mut individuals = Vec::new();
    let mut families = Vec::new();

    // Track current context for level-1 tags.
    enum IndividualContext {
        Birth,
        Death,
    }

    enum Context {
        Individual {
            idx: usize,
            sub: Option<IndividualContext>,
        },
        Family(usize),
        None,
    }
    let mut context = Context::None;

    for (idx, raw_line) in input.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        let mut parts = line.splitn(3, ' ');
        let level_str = parts.next().unwrap_or_default();
        let level: u32 = level_str
            .parse()
            .map_err(|source| ParseError::InvalidLevel {
                line: line_no,
                source,
            })?;

        let second = parts
            .next()
            .ok_or(ParseError::MissingTag { line: line_no })?;

        // GEDCOM allows an optional ID token between level and tag.
        let (xref, tag, value) = if second.starts_with('@') && second.ends_with('@') {
            let tag = parts
                .next()
                .ok_or(ParseError::MissingTag { line: line_no })?;
            let value = parts.next().unwrap_or("").trim().to_string();
            (Some(second.trim_matches('@').to_string()), tag, value)
        } else {
            let tag = second;
            let value = parts.next().unwrap_or("").trim().to_string();
            (None, tag, value)
        };

        match (level, tag) {
            (0, "INDI") => {
                let id = xref.ok_or(ParseError::MissingIndividualId { line: line_no })?;
                individuals.push(Individual {
                    id,
                    name: None,
                    birth: None,
                    death: None,
                });
                context = Context::Individual {
                    idx: individuals.len() - 1,
                    sub: None,
                };
            }
            (0, "FAM") => {
                let id = xref.ok_or(ParseError::MissingFamilyId { line: line_no })?;
                families.push(Family {
                    id,
                    husband: None,
                    wife: None,
                    children: Vec::new(),
                });
                context = Context::Family(families.len() - 1);
            }
            (1, "NAME") => {
                if let Context::Individual { idx, .. } = context {
                    individuals[idx].name = Some(value);
                    context = Context::Individual { idx, sub: None };
                } else {
                    return Err(ParseError::OrphanTag {
                        line: line_no,
                        tag: tag.to_string(),
                    });
                }
            }
            (1, "BIRT") => {
                if let Context::Individual { idx, .. } = context {
                    context = Context::Individual {
                        idx,
                        sub: Some(IndividualContext::Birth),
                    };
                } else {
                    return Err(ParseError::OrphanTag {
                        line: line_no,
                        tag: tag.to_string(),
                    });
                }
            }
            (1, "DEAT") => {
                if let Context::Individual { idx, .. } = context {
                    context = Context::Individual {
                        idx,
                        sub: Some(IndividualContext::Death),
                    };
                } else {
                    return Err(ParseError::OrphanTag {
                        line: line_no,
                        tag: tag.to_string(),
                    });
                }
            }
            (1, "HUSB") => {
                if let Context::Family(idx) = context {
                    families[idx].husband = Some(value.trim_matches('@').to_string());
                    context = Context::Family(idx);
                } else {
                    return Err(ParseError::OrphanTag {
                        line: line_no,
                        tag: tag.to_string(),
                    });
                }
            }
            (1, "WIFE") => {
                if let Context::Family(idx) = context {
                    families[idx].wife = Some(value.trim_matches('@').to_string());
                    context = Context::Family(idx);
                } else {
                    return Err(ParseError::OrphanTag {
                        line: line_no,
                        tag: tag.to_string(),
                    });
                }
            }
            (1, "CHIL") => {
                if let Context::Family(idx) = context {
                    families[idx]
                        .children
                        .push(value.trim_matches('@').to_string());
                    context = Context::Family(idx);
                } else {
                    return Err(ParseError::OrphanTag {
                        line: line_no,
                        tag: tag.to_string(),
                    });
                }
            }
            (2, "DATE") => match &mut context {
                Context::Individual {
                    idx,
                    sub: Some(IndividualContext::Birth),
                } => {
                    let event = individuals[*idx].birth.get_or_insert(Event {
                        date: None,
                        place: None,
                    });
                    event.date = Some(value);
                }
                Context::Individual {
                    idx,
                    sub: Some(IndividualContext::Death),
                } => {
                    let event = individuals[*idx].death.get_or_insert(Event {
                        date: None,
                        place: None,
                    });
                    event.date = Some(value);
                }
                _ => {
                    return Err(ParseError::OrphanTag {
                        line: line_no,
                        tag: tag.to_string(),
                    });
                }
            },
            (2, "PLAC") => match &mut context {
                Context::Individual {
                    idx,
                    sub: Some(IndividualContext::Birth),
                } => {
                    let event = individuals[*idx].birth.get_or_insert(Event {
                        date: None,
                        place: None,
                    });
                    event.place = Some(value);
                }
                Context::Individual {
                    idx,
                    sub: Some(IndividualContext::Death),
                } => {
                    let event = individuals[*idx].death.get_or_insert(Event {
                        date: None,
                        place: None,
                    });
                    event.place = Some(value);
                }
                _ => {
                    return Err(ParseError::OrphanTag {
                        line: line_no,
                        tag: tag.to_string(),
                    });
                }
            },
            _ => {
                // For now ignore other tags/levels.
            }
        }
    }

    Ok(GedcomData {
        individuals,
        families,
    })
}

pub fn load_gedcom(path: impl AsRef<Path>) -> Result<GedcomData, LoadError> {
    let contents = fs::read_to_string(path)?;
    Ok(parse_gedcom(&contents)?)
}

pub fn load_store(path: impl AsRef<Path>) -> Result<GedcomStore, LoadError> {
    let file = fs::File::open(path)?;
    let data: GedcomData = serde_json::from_reader(file)?;
    Ok(GedcomStore::from_data(data))
}
impl GedcomStore {
    pub fn from_data(data: GedcomData) -> Self {
        let individuals = data
            .individuals
            .into_iter()
            .map(|ind| (ind.id.clone(), ind))
            .collect();
        let families = data
            .families
            .into_iter()
            .map(|fam| (fam.id.clone(), fam))
            .collect();
        Self {
            individuals,
            families,
        }
    }

    pub fn get_individual(&self, id: &str) -> Option<&Individual> {
        self.individuals.get(id)
    }

    pub fn get_family(&self, id: &str) -> Option<&Family> {
        self.families.get(id)
    }

    pub fn families(&self) -> impl Iterator<Item = &Family> {
        self.families.values()
    }

    pub fn individuals(&self) -> impl Iterator<Item = &Individual> {
        self.individuals.values()
    }

    pub fn insert_individual(&mut self, individual: Individual) -> Result<(), StoreError> {
        if self.individuals.contains_key(&individual.id) {
            return Err(StoreError::DuplicateIndividual(individual.id));
        }
        self.individuals.insert(individual.id.clone(), individual);
        Ok(())
    }

    pub fn insert_family(&mut self, family: Family) -> Result<(), StoreError> {
        if self.families.contains_key(&family.id) {
            return Err(StoreError::DuplicateFamily(family.id));
        }
        self.families.insert(family.id.clone(), family);
        Ok(())
    }

    pub fn to_data(&self) -> GedcomData {
        GedcomData {
            individuals: self.individuals.values().cloned().collect(),
            families: self.families.values().cloned().collect(),
        }
    }

    pub fn save_to_path(&self, path: impl AsRef<Path>) -> Result<(), std::io::Error> {
        let data = self.to_data();
        let mut file = fs::File::create(path)?;
        serde_json::to_writer_pretty(&mut file, &data)
            .map_err(|err| std::io::Error::new(ErrorKind::Other, err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile;

    #[test]
    fn parses_minimal_individuals_and_family() {
        let input = r#"
        0 @I1@ INDI
        1 NAME John /Doe/
        1 BIRT
        2 DATE 1 JAN 1900
        2 PLAC Springfield
        0 @I2@ INDI
        1 NAME Jane /Doe/
        1 DEAT
        2 DATE 2 FEB 2000
        0 @F1@ FAM
        1 HUSB @I1@
        1 WIFE @I2@
        1 CHIL @I3@
        "#;

        let data = parse_gedcom(input).expect("should parse");

        assert_eq!(
            data.individuals,
            vec![
                Individual {
                    id: "I1".into(),
                    name: Some("John /Doe/".into()),
                    birth: Some(Event {
                        date: Some("1 JAN 1900".into()),
                        place: Some("Springfield".into())
                    }),
                    death: None
                },
                Individual {
                    id: "I2".into(),
                    name: Some("Jane /Doe/".into()),
                    birth: None,
                    death: Some(Event {
                        date: Some("2 FEB 2000".into()),
                        place: None
                    })
                }
            ]
        );
        assert_eq!(
            data.families,
            vec![Family {
                id: "F1".into(),
                husband: Some("I1".into()),
                wife: Some("I2".into()),
                children: vec!["I3".into()]
            }]
        );
    }

    #[test]
    fn errors_on_missing_individual_id() {
        let input = r#"
        0 INDI
        1 NAME Unknown
        "#;

        let err = parse_gedcom(input).expect_err("should fail");
        assert!(matches!(err, ParseError::MissingIndividualId { .. }));
    }

    #[test]
    fn errors_on_orphan_tag() {
        let input = r#"
        1 NAME NoContext
        "#;

        let err = parse_gedcom(input).expect_err("should fail");
        assert!(matches!(err, ParseError::OrphanTag { .. }));
    }

    #[test]
    fn errors_on_invalid_level() {
        let input = r#"
        x @I1@ INDI
        "#;

        let err = parse_gedcom(input).expect_err("should fail");
        assert!(matches!(err, ParseError::InvalidLevel { .. }));
    }

    #[test]
    fn loads_from_path() {
        let mut tmp = tempfile::NamedTempFile::new().expect("create temp file");
        tmp.write_all(
            br#"
            0 @I1@ INDI
            1 NAME Test /User/
            "#,
        )
        .expect("write temp file");

        let data = load_gedcom(tmp.path()).expect("should load");
        assert_eq!(
            data.individuals,
            vec![Individual {
                id: "I1".into(),
                name: Some("Test /User/".into()),
                birth: None,
                death: None
            }]
        );
    }

    #[test]
    fn errors_on_date_without_birth_context() {
        let input = r#"
        0 @I1@ INDI
        1 NAME Test /User/
        2 DATE 1 JAN 2000
        "#;

        let err = parse_gedcom(input).expect_err("should fail");
        assert!(matches!(err, ParseError::OrphanTag { .. }));
    }

    #[test]
    fn indexes_individuals_and_families() {
        let data = GedcomData {
            individuals: vec![Individual {
                id: "I1".into(),
                name: Some("Indexed".into()),
                birth: None,
                death: None,
            }],
            families: vec![Family {
                id: "F1".into(),
                husband: Some("I1".into()),
                wife: None,
                children: vec![],
            }],
        };

        let store = GedcomStore::from_data(data);
        let individual = store.get_individual("I1").expect("individual present");
        assert_eq!(individual.name.as_deref(), Some("Indexed"));
        let family = store.get_family("F1").expect("family present");
        assert_eq!(family.husband.as_deref(), Some("I1"));
        assert_eq!(store.individuals().count(), 1);
        assert_eq!(store.families().count(), 1);
    }

    #[test]
    fn inserts_unique_individuals() {
        let mut store = GedcomStore::from_data(GedcomData {
            individuals: vec![],
            families: vec![],
        });

        store
            .insert_individual(Individual {
                id: "I1".into(),
                name: Some("First".into()),
                birth: None,
                death: None,
            })
            .expect("insert succeeds");

        let err = store
            .insert_individual(Individual {
                id: "I1".into(),
                name: Some("Duplicate".into()),
                birth: None,
                death: None,
            })
            .expect_err("should reject duplicate");
        assert!(matches!(err, StoreError::DuplicateIndividual(id) if id == "I1"));
    }

    #[test]
    fn inserts_unique_families() {
        let mut store = GedcomStore::from_data(GedcomData {
            individuals: vec![],
            families: vec![],
        });

        store
            .insert_family(Family {
                id: "F1".into(),
                husband: None,
                wife: None,
                children: vec![],
            })
            .expect("insert succeeds");

        let err = store
            .insert_family(Family {
                id: "F1".into(),
                husband: None,
                wife: None,
                children: vec![],
            })
            .expect_err("should reject duplicate family");
        assert!(matches!(err, StoreError::DuplicateFamily(id) if id == "F1"));
    }

    #[test]
    fn saves_store_to_path() {
        let store = GedcomStore::from_data(GedcomData {
            individuals: vec![Individual {
                id: "I1".into(),
                name: Some("Save".into()),
                birth: None,
                death: None,
            }],
            families: vec![],
        });

        let tmp = tempfile::NamedTempFile::new().expect("temp");
        store.save_to_path(tmp.path()).expect("save");

        let contents = std::fs::read_to_string(tmp.path()).expect("read");
        assert!(contents.contains("\"I1\""));
        assert!(contents.contains("Save"));
    }

    #[test]
    fn loads_store_from_snapshot() {
        let store = GedcomStore::from_data(GedcomData {
            individuals: vec![Individual {
                id: "I1".into(),
                name: Some("Saved".into()),
                birth: None,
                death: None,
            }],
            families: vec![Family {
                id: "F1".into(),
                husband: Some("I1".into()),
                wife: None,
                children: vec![],
            }],
        });

        let tmp = tempfile::NamedTempFile::new().expect("temp");
        store.save_to_path(tmp.path()).expect("save");

        let loaded = load_store(tmp.path()).expect("load snapshot");
        let indiv = loaded.get_individual("I1").expect("individual present");
        assert_eq!(indiv.name.as_deref(), Some("Saved"));
        let fam = loaded.get_family("F1").expect("family present");
        assert_eq!(fam.husband.as_deref(), Some("I1"));
    }
}
