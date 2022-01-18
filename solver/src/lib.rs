//!
//! 

extern crate plbot_base;
extern crate mediawiki;

mod util;
mod error;
#[cfg(feature="mwapi")]
mod api;

use crate::error::SolveError;
use plbot_base::{ir::RegID, bot::APIAssertType};
use util::{get_set_1, get_set_2};

use plbot_base::{Query, ir::Instruction};

use std::collections::{HashSet, HashMap};
use mediawiki::{title::Title, api::Api, api::NamespaceID};

pub(crate) type Register = HashMap<RegID, HashSet<Title>>;

#[cfg(feature="mwapi")]
pub async fn solve_api(query: &Query, api: &Api, assert: Option<APIAssertType>) -> Result<HashSet<Title>, SolveError> {
    // prepare a mock register pool using HashMap
    let mut reg: Register = HashMap::new();
    for inst in query.0.iter() {
        match inst {
            Instruction::And { dest, op1, op2 } => {
                let (set1, set2) = get_set_2(&reg, op1, op2)?;
                let intersect: HashSet<Title> = set1.intersection(set2).cloned().collect();
                reg.insert(*dest, intersect);
            },
            Instruction::Or { dest, op1, op2 } => {
                let (set1, set2) = get_set_2(&reg, op1, op2)?;
                let union: HashSet<Title> = set1.union(set2).cloned().collect();
                reg.insert(*dest, union);
            },
            Instruction::Exclude { dest, op1, op2 } => {
                let (set1, set2) = get_set_2(&reg, op1, op2)?;
                let diff: HashSet<Title> = set1.difference(set2).cloned().collect();
                reg.insert(*dest, diff);
            },
            Instruction::Xor { dest, op1, op2 } => {
                let (set1, set2) = get_set_2(&reg, op1, op2)?;
                let xor: HashSet<Title> = set1.symmetric_difference(set2).cloned().collect();
                reg.insert(*dest, xor);
            },
            Instruction::LinkTo { dest, op, cs } => {
                let set = get_set_1(&reg, op)?;
                if set.is_empty() {
                    reg.insert(*dest, HashSet::new());
                } else if set.len() > 1 {
                    return Err(SolveError::QueryForMultiplePages);
                } else {
                    let mut result_set: HashSet<Title> = HashSet::new();
                    for t in set.iter() {
                        let res_one = api::get_backlinks_one(t, api, assert, cs.ns.as_ref(), true).await?;
                        result_set.extend(res_one);
                    }
                    reg.insert(*dest, result_set);
                }
            },
            Instruction::InCat { dest, op, cs } => {
                let set = get_set_1(&reg, op)?;
                if set.is_empty() {
                    reg.insert(*dest, HashSet::new());
                } else if set.len() > 1 {
                    return Err(SolveError::QueryForMultiplePages);
                } else {
                    let sub_limit = cs.depth.unwrap_or(0);
                    let mut result_set: HashSet<Title> = HashSet::new();
                    for t in set.iter() {
                        let res_one = api::get_category_members_one(t, api, assert, cs.ns.as_ref(), sub_limit).await?;
                        result_set.extend(res_one);
                    }
                    reg.insert(*dest, result_set);
                }
            },
            Instruction::Toggle { dest, op } => {
                let set = get_set_1(&reg, op)?;
                let title_set: HashSet<Title> = set.iter().cloned().map(|title| title.into_toggle_talk()).collect();
                reg.insert(*dest, title_set);
            },
            Instruction::Prefix { dest, op } => {
                let set = get_set_1(&reg, op)?;
                if set.is_empty() {
                    reg.insert(*dest, HashSet::new());
                } else if set.len() > 1 {
                    return Err(SolveError::QueryForMultiplePages);
                } else {
                    let mut result_set: HashSet<Title> = HashSet::new();
                    for t in set.iter() {
                        let res_one = api::get_prefix_index_one(t, api, assert).await?;
                        result_set.extend(res_one);
                    }
                    reg.insert(*dest, result_set);
                }
            },
            Instruction::Set { dest, titles, cs } => {
                let mut title_set: HashSet<Title> = HashSet::new();
                for t in titles {
                    let title: Title = Title::new_from_full(t, api);
                    if let Some(nss) = &cs.ns {
                        if !nss.contains(&title.namespace_id()) {
                            continue;
                        }
                    }
                    title_set.insert(title);
                }
                reg.insert(*dest, title_set);
            },
            Instruction::Nop { dest, op } => {
                let set = get_set_1(&reg, op)?;
                let copiedset = set.clone();
                reg.insert(*dest, copiedset);
            },
        }
    }

    let result = get_set_1(&reg, &query.1)?;
    Ok(result.clone())
}
