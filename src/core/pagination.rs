//! Generic pagination primitive (clasp `fetchWithPages`, `utils.ts:186-214`;
//! spec §7.3, §2.1). Typed API adapters (Task 4) call this with their
//! endpoint-specific closures; Service Usage endpoints override the page
//! size (200) and result cap (10000).

use crate::error::CrspError;

/// Default page size (clasp `pageOptionsWithDefaults`).
pub const DEFAULT_PAGE_SIZE: usize = 100;

/// Default page limit (clasp `pageOptionsWithDefaults`).
pub const DEFAULT_MAX_PAGES: usize = 10;

/// Service Usage `services.list` page size (spec §2.1: list-apis).
pub const SERVICE_USAGE_PAGE_SIZE: usize = 200;

/// Service Usage `services.list` result cap (spec §2.1: list-apis).
pub const SERVICE_USAGE_MAX_RESULTS: usize = 10000;

/// Knobs for [`fetch_pages`]. clasp defaults: page size 100, max pages 10,
/// and an effectively unlimited result cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageOptions {
    pub page_size: usize,
    pub max_pages: usize,
    pub max_results: usize,
}

impl Default for PageOptions {
    fn default() -> Self {
        Self {
            page_size: DEFAULT_PAGE_SIZE,
            max_pages: DEFAULT_MAX_PAGES,
            max_results: usize::MAX,
        }
    }
}

/// One fetched page: its results and the token for the next page, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<T> {
    pub results: Vec<T>,
    pub page_token: Option<String>,
}

/// Aggregated results with clasp's `partialResults` flag (spec §7.3): true
/// when the loop stopped on a page/result limit — a normal outcome, not an
/// error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PagedResults<T> {
    pub results: Vec<T>,
    pub partial_results: bool,
}

/// Fetches pages until the token is exhausted or a limit is reached.
/// Mid-page failures returned by `fetch` propagate immediately (never
/// swallowed, and without partial results — spec §7.3).
///
/// The `Send` bounds keep the whole call chain spawnable so async task
/// contexts (MCP tool handlers, tokio tasks) can page through any typed
/// API adapter — there is exactly one pagination loop (spec §7.3).
pub async fn fetch_pages<T, F, Fut>(
    mut fetch: F,
    options: PageOptions,
) -> Result<PagedResults<T>, CrspError>
where
    F: FnMut(usize, Option<String>) -> Fut + Send,
    Fut: std::future::Future<Output = Result<Page<T>, CrspError>> + Send,
{
    let PageOptions {
        page_size,
        max_pages,
        max_results,
    } = options;

    let mut results: Vec<T> = Vec::new();
    let mut page_token: Option<String> = None;
    let mut page_count = 0usize;

    loop {
        let page = fetch(page_size, page_token.take()).await?;
        results.extend(page.results);
        page_count += 1;
        page_token = page.page_token;
        if page_token.is_none() || page_count >= max_pages || results.len() >= max_results {
            break;
        }
    }

    if results.len() > max_results {
        results.truncate(max_results);
        return Ok(PagedResults {
            results,
            partial_results: true,
        });
    }

    Ok(PagedResults {
        results,
        partial_results: page_token.is_some(),
    })
}
