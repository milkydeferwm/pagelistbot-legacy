//! This module performs actions using MediaWiki API
//! 

#![cfg(feature="mwapi")]

use crate::{NamespaceID, util, error::SolveError};
use std::collections::{HashSet, VecDeque};
use mediawiki::{api::Api, title::Title};
use plbot_base::{bot::APIAssertType, ir::{DepthNum, RedirectStrategy}};

/// Retrives the backlink for one page.
/// 
/// "Backlink" refers to internal links and redirects. Transclusions (common for templates) are not considered as backlinks.
/// For example, if template B has a link to page A, and template B is transcluded into page C, then C is a backlink to A but B is not.
/// 
/// `title`: The title of the page.
/// 
/// `api`: The MediaWiki API instance.
/// 
/// `assert`: The identity to assert for when using MediaWiki API. If set to `None`, won't apply assertion.
/// 
/// `ns`: Namespace filter. If set to `None`, then the result is not filtered by namespace.
/// 
/// `level_2`: Whether to include pages that links to a redirect of `title`.
/// 
/// `redirect_strat`: The redirect strategy to use when querying.
pub(crate) async fn get_backlinks_one(title: &Title, api: &Api, assert: Option<APIAssertType>, ns: Option<&HashSet<NamespaceID>>, level_2: bool, redirect_strat: RedirectStrategy) -> Result<HashSet<Title>, SolveError> {
    let elem_name = title.full_pretty(&api);
    if elem_name.is_none() {
        Ok(HashSet::new())
    } else {
        let mut params = api.params_into(&[
            ("utf8", "1"),
            ("action", "query"),
            ("list", "backlinks"),
            ("bltitle", &elem_name.unwrap()),
            ("bllimit", "max"),
            ("blfilterredir", redirect_strat.to_string().as_str()),
        ]);
        util::insert_assert_param(&mut params, assert);
        if level_2 {
            // If level_2 is `true`, we cannot filter namespaces in the query. Here is the reason.
            // Suppose we have an inter-namespace redirect, for example,
            // [[w:zh:LTA:KAGE]] (main) -> [[w:zh:Wikipedia:持续出没的破坏者/User:影武者]] (Project)
            // and there are pages in "Project" namespace that link to [[LTA:KAGE]].
            // If we add "blnamespace=4" ("Project")  to the query, we cannot access these pages,
            // because the link target [[LTA:KAGE]] (main) is filtered out.
            params.insert("blredirect".to_string(), "1".to_string());
        } else {
            // We can safely apply namespace restrictions
            if let Some(ns_list) = ns {
                params.insert("blnamespace".to_string(), util::concat_params(ns_list));
            }
        }
        let res = api.get_query_api_json_all(&params).await?;
        util::detect_api_failure(&res)?;
        // Api::result_array_to_titles cannot handle nested redirect Titles well...
        // Maybe an issue should be filed
        let result_should_have_redirect = match redirect_strat {
            RedirectStrategy::NoRedirect => false,
            _ => true,
        };
        let mut title_vec = result_array_to_titles_ex(&res, result_should_have_redirect);
        // Need to filter by namespace...
        if level_2 {
            if let Some(ns_list) = ns {
                title_vec.retain(|title| ns_list.contains(&title.namespace_id()));
            }
        }
        let title_set = HashSet::from_iter(title_vec.into_iter());
        Ok(title_set)
    }
}

/// Retrives the members of one category. Dive into subcategories if possible.
/// Unfortunately, MediaWiki API does not provide any option to filter out redirects.
/// 
/// `title`: The title of the category.
/// 
/// `api`: The MediaWiki API instance.
/// 
/// `assert`: The identity to assert for when using MediaWiki API. If set to `None`, won't apply assertion.
/// 
/// `ns`: Namespace filter. If set to `None`, then the result is not filtered by namespace.
/// 
/// `depth`: Maximum depth we should dive into. The category `title` sits at level 0, its sub categories sit at level 1, and so on. If `depth` is negative, then **every subcategory** in the hierarchy will be visited, which could be costly.
pub(crate) async fn get_category_members_one(title: &Title, api: &Api, assert: Option<APIAssertType>, ns: Option<&HashSet<NamespaceID>>, depth: DepthNum) -> Result<HashSet<Title>, SolveError> {
    // Due to miser mode, we need to do some preparations to cs.
    let mut ns_clone = ns.cloned();
    let mut result_has_ns_category: bool = true;
    let mut result_has_ns_file: bool = true;
    if let Some(ns_list) = ns_clone.as_mut() {
        result_has_ns_category = ns_list.remove(&plbot_base::NS_CATEGORY);
        result_has_ns_file = ns_list.remove(&plbot_base::NS_FILE);
    }
    // Do a bfs search of category tree (perhaps graph).
    // Looks like it is possible to construct a "sub category loop".
    // In fact, [[w:en:Category:Recursion]] is indef full protected to
    // prevent editors from adding itself to its sub categories.
    let mut result_set: HashSet<Title> = HashSet::new();
    let mut visited_cats: HashSet<Title> = HashSet::new();
    visited_cats.insert(title.to_owned());
    let mut visit_cat_queue: VecDeque<(Title, DepthNum)> = VecDeque::new();
    visit_cat_queue.push_back((title.to_owned(), 0));
    while let Some((this_cat, this_depth)) = visit_cat_queue.pop_front() {
        if this_cat.namespace_id() != plbot_base::NS_CATEGORY {
            return Err(SolveError::NotCategory);
        }
        let cat_name = this_cat.full_pretty(api).unwrap();
        let mut params = api.params_into(&[
            ("utf8", "1"),
            ("action", "query"),
            ("list", "categorymembers"),
            ("cmtitle", &cat_name),
            ("cmlimit", "max"),
        ]);
        util::insert_assert_param(&mut params, assert);
        // determine what cmtype and cmnamespace should we insert
        let mut cmtype: Vec<String> = Vec::new();
        let mut cmnamespace: HashSet<NamespaceID> = HashSet::new();
        // If we still have some namespaces left in `ns_clone`...
        if let Some(ns_list) = &ns_clone {
            cmtype.push("page".to_string());
            cmnamespace.extend(ns_list);
        }
        if result_has_ns_file {
            cmtype.push("file".to_string());
            cmnamespace.insert(plbot_base::NS_FILE);
        }
        // If we still need to find subcats, or `result_has_ns_category`...
        if result_has_ns_category || (depth < 0 || this_depth < depth) {
            cmtype.push("subcat".to_string());
            cmnamespace.insert(plbot_base::NS_CATEGORY);
        }
        params.insert("cmnamespace".to_string(), util::concat_params(&cmnamespace));
        params.insert("cmtype".to_string(), cmtype.join("|"));
        // fetch results
        let res = api.get_query_api_json_all(&params).await?;
        util::detect_api_failure(&res)?;
        let mut title_vec = Api::result_array_to_titles(&res);
        if depth < 0 || this_depth < depth {
            // filter out subcategories from title_vec, and add to visit queue
            for sub in title_vec.iter().filter(|&t| t.namespace_id() == plbot_base::NS_CATEGORY) {
                if !visited_cats.contains(sub) {
                    visited_cats.insert(sub.to_owned());
                    visit_cat_queue.push_back((sub.to_owned(), this_depth + 1));
                }
            }
        }
        if !result_has_ns_category {
            title_vec.retain(|f| f.namespace_id() != plbot_base::NS_CATEGORY);
        }
        result_set.extend(title_vec);
    }
    Ok(result_set)
}

/// Retrives the pages with the given prefix. That is how [[Special:PrefixIndex]] works.
/// 
/// This function does not need a namespace constraint, because it is implied by the prefix.
/// However, we still provide it. If the page's namespace does not exist in the requested namespaces,
/// an empty set is directly returned without any API requests.
/// 
/// `title`: The title of the page.
/// 
/// `api`: The MediaWiki API instance.
/// 
/// `assert`: The identity to assert for when using MediaWiki API. If set to `None`, won't apply assertion.
/// 
/// `ns`: Namespace filter. If set to `None`, then the result is not filtered by namespace.
/// 
/// `redirect_strat`: The redirect strategy to use when querying.
pub(crate) async fn get_prefix_index_one(title: &Title, api: &Api, assert: Option<APIAssertType>, ns: Option<&HashSet<NamespaceID>>, redirect_strat: RedirectStrategy) -> Result<HashSet<Title>, SolveError> {
    let title_ns_id = title.namespace_id();
    if let Some(ns_list) = ns {
        if !ns_list.contains(&title_ns_id) {
            return Ok(HashSet::new());
        }
    }
    let mut params = api.params_into(&[
        ("utf8", "1"),
        ("action", "query"),
        ("list", "allpages"),
        ("apprefix", title.pretty()),
        ("apnamespace", NamespaceID::to_string(&title_ns_id).as_str()),
        ("aplimit", "max"),
        ("apfilterredir", redirect_strat.to_string().as_str()),
    ]);
    util::insert_assert_param(&mut params, assert);
    let res = api.get_query_api_json_all(&params).await?;
    util::detect_api_failure(&res)?;
    let title_vec = Api::result_array_to_titles(&res);
    let title_set = HashSet::from_iter(title_vec.into_iter());
    Ok(title_set)
}

/// Retrives the pages that embeds a specific page.
/// 
/// Any page that transcludes this page (either via template redirects, or template itself uses this page) is considered embeds this page.
/// 
/// `title`: The title of the page.
/// 
/// `api`: The MediaWiki API instance.
/// 
/// `assert`: The identity to assert for when using MediaWiki API. If set to `None`, won't apply assertion.
/// 
/// `ns`: Namespace filter. If set to `None`, then the result is not filtered by namespace.
/// 
/// `redirect_strat`: The redirect strategy to use when querying. This is useful if a redirect page itself transcludes this page.
pub(crate) async fn get_embed_one(title: &Title, api: &Api, assert: Option<APIAssertType>, ns: Option<&HashSet<NamespaceID>>, redirect_strat: RedirectStrategy) -> Result<HashSet<Title>, SolveError> {
    let elem_name = title.full_pretty(&api);
    if elem_name.is_none() {
        Ok(HashSet::new())
    } else {
        let mut params = api.params_into(&[
            ("utf8", "1"),
            ("action", "query"),
            ("list", "embeddedin"),
            ("eititle", &elem_name.unwrap()),
            ("eilimit", "max"),
            ("eifilterredir", redirect_strat.to_string().as_str()),
        ]);
        if let Some(ns_list) = ns {
            params.insert("einamespace".to_string(), util::concat_params(ns_list));
        }
        util::insert_assert_param(&mut params, assert);
        let res = api.get_query_api_json_all(&params).await?;
        util::detect_api_failure(&res)?;
        let title_vec = Api::result_array_to_titles(&res);
        let title_set = HashSet::from_iter(title_vec.into_iter());
        Ok(title_set)
    }
}

/// Internal
/// This is an extension to Api::result_array_to_titles.
/// It offers an option to filter out redirects.
fn result_array_to_titles_ex(data: &serde_json::Value, include_redirect: bool) -> Vec<Title> {
    // See if it's the "root" of the result, then try each sub-object separately
    if let Some(obj) = data.as_object() {
        obj.iter()
            .flat_map(|(_k, v)| result_array_to_titles_ex(&v, include_redirect))
            .collect()
    } else if let Some(arr) = data.as_array() {
        let mut title_vec: Vec<Title> = vec![];
        for item in arr.iter() {
            if item.get("redirect").is_some() {
                // This item is a redirect
                // Should look into sub, if possible
                if let Some(subs) = item.get("redirlinks") {
                    let mut res = result_array_to_titles_ex(subs, include_redirect);
                    title_vec.append(&mut res);
                }
                if include_redirect {
                    title_vec.push(Title::new_from_api_result(&item));
                }
            } else {
                title_vec.push(Title::new_from_api_result(&item));
            }
        };
        title_vec
    } else {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    #[test]
    fn test_result_array_to_titles_ex() {
        use crate::api::result_array_to_titles_ex;
        let mock_data = json!({
            "batchcomplete": "",
            "limits": {
                "backlinks": 250
            },
            "query": {
                "backlinks": [
                    {
                        "pageid": 468116,
                        "ns": 4,
                        "title": "Wikipedia:持续出没的破坏者"
                    },
                    {
                        "pageid": 526249,
                        "ns": 4,
                        "title": "Wikipedia:KAGE",
                        "redirect": "",
                        "redirlinks": [
                            {
                                "pageid": 502437,
                                "ns": 4,
                                "title": "Wikipedia:当前的破坏/存档/2007年"
                            }
                        ]
                    }
                ]
            }
        });
        assert_eq!(result_array_to_titles_ex(&mock_data, true).len(), 3);
        assert_eq!(result_array_to_titles_ex(&mock_data, false).len(), 2);
    }
}