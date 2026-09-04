//! Integration tests: spawn the real server on an ephemeral port and drive it
//! over HTTP with reqwest.

use futures::future::join_all;
use reqwest::Method;
use reqwest::StatusCode;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use takomo::store::ShareKind;

mod common;
use common::TestApp;

// ---------------------------------------------------------------------------

#[tokio::test]
async fn healthz_open_everything_else_authed() {
    let app = TestApp::spawn().await;
    let resp = app.request(Method::GET, "/healthz").send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = app
        .request(Method::GET, "/v1/tickets")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["code"], "auth.missing");

    let resp = app
        .request(Method::GET, "/v1/tickets")
        .bearer_auth("tk_bogusbogusbogusbogus1")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn inbox_and_board_pages_served_unauthenticated() {
    let app = TestApp::spawn().await;
    for path in PAGE_ROUTES {
        let resp = app.request(Method::GET, path).send().await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "{path} should serve");
        let body = resp.text().await.unwrap();
        assert_app_shell(path, &body);
        // The octopus, on every route.
        assert!(body.contains("rel=\"icon\""), "{path} links a favicon");
    }

    // The per-surface title used to live in each document's <head>; with one
    // document it is set from the path at runtime, so it is asserted in the
    // bundle instead. Losing it would leave every tab reading "takomo · board".
    let bundle = app.eager_bundle().await;
    for title in [
        "takomo · board",
        "takomo · inbox",
        "takomo · initiatives",
        "takomo · schedules",
        "takomo · settings",
        "takomo · verification",
        "takomo · environments",
    ] {
        assert!(bundle.contains(title), "no document title for `{title}`");
    }
    assert!(
        bundle.contains("/questions"),
        "the inbox talks to the questions API"
    );
}

/// The ticket filter on `/board` and `/inbox` is client-side, so its contract has
/// two halves that break independently: the API must keep exposing the fields the
/// filter reads, and the pages must keep shipping the control that reads them.
/// A server-side rename of `parent`/`ticket` would silently degrade both filters to
/// "matches nothing" without failing any other test.
#[tokio::test]
async fn ticket_filter_contract_on_board_and_inbox() {
    let app = TestApp::spawn().await;

    // An epic with a child, so the board's subtree filter has a chain to walk:
    // filtering by the epic must keep its subtasks visible, not orphan them.
    let epic = app.create_typed("Billing revamp", "epic", None).await;
    let child = app
        .create_typed("Migrate off billing_v1", "task", Some(&epic))
        .await;
    let other = app.create_typed("Unrelated work", "task", None).await;

    // Advisory: needs no claim/fence and parks nothing, so the ticket stays put.
    app.ask(
        &app.admin,
        json!({
            "ticket": child,
            "mode": "advisory",
            "kind": "confirm",
            "title": "Drop the legacy column?",
        }),
    )
    .await;

    // ---- half 1: the fields the filters read ----
    // The board walks `parent` upward to decide subtree membership.
    let (_, list) = app
        .get(&app.admin, "/v1/tickets?project=tp&limit=200")
        .await;
    let listed = list["items"]
        .as_array()
        .expect("items array")
        .iter()
        .find(|t| t["id"] == json!(child))
        .expect("child ticket is listed");
    assert_eq!(
        listed["parent"],
        json!(epic),
        "tickets must expose `parent`; the board's subtree filter walks it"
    );

    // The inbox groups the queue by `ticket` — that field both populates its
    // picker and decides which questions a chosen ticket shows.
    let (_, qs) = app
        .get(&app.admin, "/v1/questions?project=tp&status=open")
        .await;
    assert!(
        qs["items"]
            .as_array()
            .expect("items array")
            .iter()
            .any(|x| x["ticket"] == json!(child)),
        "questions must expose `ticket`; the inbox filter groups on it"
    );

    // The same narrowing over HTTP, so an integrator filtering server-side gets
    // what the page computes client-side.
    let (_, only) = app
        .get(
            &app.admin,
            &format!("/v1/questions?ticket={child}&status=open"),
        )
        .await;
    let items = only["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1, "?ticket= narrows to that ticket: {only}");
    assert_eq!(items[0]["ticket"], json!(child));
    let (_, none) = app
        .get(
            &app.admin,
            &format!("/v1/questions?ticket={other}&status=open"),
        )
        .await;
    assert!(
        none["items"].as_array().expect("items array").is_empty(),
        "?ticket= on a ticket with no questions returns nothing: {none}"
    );

    // ---- half 2: the control that reads them ----
    // Both surfaces mount a typeahead — /board's since takomo-fo1j, /inbox's
    // since takomo-4io8 — over the same two fields asserted above. They are one
    // bundle now, so this reads it once instead of fetching two documents that
    // would be byte-identical.
    let bundle = app.eager_bundle().await;
    for (surface, control, wiring) in [
        // JSX compiles `id="tickfilter"` to a prop and the subtree walk is a
        // module whose local names the minifier renames. The stable signals are
        // the mount id and the `/tickets` fetch the filter reads.
        ("board", "tickfilter", "/tickets"),
        // The inbox keeps the same two names: the control's id, and the
        // `visible()` set the folder split and the counts both read from.
        ("inbox", "tickpick", "visible"),
    ] {
        assert!(
            bundle.contains(control),
            "the {surface} ticket-filter control ('{control}') is missing from the bundle"
        );
        assert!(
            bundle.contains(wiring),
            "the {surface} ticket filter is not wired into its render path ('{wiring}')"
        );
    }
    // Locale parity is a compile error in web/ (`defineStrings` makes EN the
    // reference shape), so this is presence rather than the old per-page count
    // of two — which counted 4 against one bundle carrying four string tables.
    assert!(
        bundle.contains("allTickets:"),
        "the `allTickets` filter label is missing from the bundle"
    );

    // The typeahead has to stay keyboard-operable: the <select> it replaced was,
    // for free. There is ONE `Typeahead` component behind all five mounts now,
    // so a regression here cannot affect one surface and spare another — which
    // is exactly why the old two-page loop is gone rather than duplicated.
    for marker in [
        // The ROLES, not the syntax that sets them.
        "combobox",              // the input announces itself as a combobox
        "aria-expanded",         // …and whether its popup is open
        "aria-activedescendant", // …and which option the arrow keys are on
        "listbox",               // the popup is a real listbox
        "option",                // with real options
        // The key NAMES, not their quoting: the minifier emits backticks
        // (`ArrowDown`), so asserting on double quotes would be asserting on
        // minifier output rather than on the control being operable.
        "ArrowDown", // arrow keys move the active option
        "Enter",     // Enter commits it
        "Escape",    // Escape dismisses the popup
        "ta-clear",  // and the selection is clearable
    ] {
        assert!(
            bundle.contains(marker),
            "the ticket typeahead must keep '{marker}' — without it the control is no \
             longer fully keyboard-operable, which the <select> it replaced was"
        );
    }
}

/// `/inbox`'s ticket filter searches ticket *titles*, not just ids (takomo-4io8),
/// and its epic grouping walks `parent`/`type`.
///
/// The titles are the whole point of the control — an id-only list is what the
/// <select> already offered — and they arrive on a request the page makes for
/// another reason entirely (the tag map). `parent` and `type` ride the same
/// request: without them the inbox cannot tell which epic a question's ticket
/// sits under, so "group by epic" would render one undifferentiated group and
/// filtering by an epic would show an EMPTY inbox — questions hang off the
/// leaves. So this pins both halves: the sparse projection actually returns all
/// five fields, and the page asks for them.
#[tokio::test]
async fn inbox_ticket_filter_has_titles_to_search() {
    let app = TestApp::spawn().await;
    let epic = app.create_typed("Lease hygiene", "epic", None).await;
    let id = app
        .create_typed("Sweep expired leases", "task", Some(&epic))
        .await;

    // The projection the inbox uses: one request carrying tags, titles and the
    // tree.
    let (_, list) = app
        .get(
            &app.admin,
            "/v1/tickets?project=tp&fields=id,title,tags,parent,type",
        )
        .await;
    let t = list["items"]
        .as_array()
        .expect("items array")
        .iter()
        .find(|t| t["id"] == json!(id))
        .expect("ticket is listed");
    assert_eq!(
        t["title"],
        json!("Sweep expired leases"),
        "the projection must return the title the filter searches: {list}"
    );
    assert_eq!(
        t["parent"],
        json!(epic),
        "…and `parent`, which the epic grouping walks upward: {list}"
    );
    assert_eq!(
        t["type"],
        json!("task"),
        "…and `type`, which is how the walk recognises the epic it stops at: {list}"
    );
    assert!(
        t.get("body").is_none(),
        "…and stay sparse — the filter needs five fields, not the whole ticket: {list}"
    );

    let body = app.eager_bundle().await;
    assert!(
        body.contains("fields=id,title,tags,parent,type"),
        "/inbox must request `title`, `parent` and `type` on the ticket fetch it \
         already makes, or the filter has nothing but ids to match on and the epic \
         grouping has no tree to walk"
    );
    // Locale parity used to be counted here — two occurrences meant a DE and an
    // EN entry. That count is meaningless against one bundle carrying all four
    // surfaces' tables, and it is also no longer this layer's job: `defineStrings`
    // makes EN the reference shape, so a missing DE key is a COMPILE error in
    // web/. What is still worth pinning here is that the key ships at all.
    assert!(
        body.contains("taTicket:"),
        "the inbox's ticket-filter label is missing from the bundle"
    );
}

/// `/board`'s tag-value filter is the *same* typeahead as its ticket filter
/// (takomo-0yl3), not a second bespoke control. That is the decision worth
/// pinning: two mount points, one `makeTypeahead`. If a later change forks them,
/// the ARIA and keyboard guarantees above stop covering the tag filter and
/// nothing else would say so.
#[tokio::test]
async fn board_tag_value_filter_reuses_the_ticket_typeahead() {
    let app = TestApp::spawn().await;
    let body = app.eager_bundle().await;

    assert!(
        body.contains("tagvalfilter"),
        "/board mounts the tag-value typeahead"
    );
    assert!(
        body.contains("tagkindsel"),
        "the tag *kind* stays a <select> — a handful of kinds needs no search"
    );
    // The invariant is "one implementation, however many callers". The old form
    // counted `function makeTypeahead(` definitions and call sites in the served
    // bytes; a bundled page renames both, so the count says nothing.
    //
    // It is now structural instead: every filter mounts the SAME
    // `web/src/components/Typeahead.tsx`, so a second copy is not something a
    // careless edit can produce — it would be a new file, and a reviewer would
    // see it. What the served page can still prove is that each mount point is
    // actually there.
    for mount in ["tickfilter", "tagvalfilter", "epicfilter", "labelfilter"] {
        assert!(
            body.contains(mount),
            "/board mounts `{mount}` — every filter that needs a combobox reuses \
             the one Typeahead component rather than growing a second"
        );
    }
    // Presence, not a count: see the note in `inbox_ticket_filter_has_titles_to_search`.
    // Locale parity is enforced by `defineStrings` at compile time in web/.
    for key in ["taTagValue:", "taLabel:"] {
        assert!(
            body.contains(key),
            "the board's `{key}` filter label is missing from the bundle"
        );
    }
}

/// The five routes that serve the application shell.
///
/// They serve the SAME document — the app is client-side routed — so a test that
/// distinguishes them by content is asserting something that no longer exists.
/// What each route still owes the caller is the shell contract below.
const PAGE_ROUTES: &[&str] = &[
    "/board",
    "/documents",
    "/verification",
    "/environments",
    "/inbox",
    "/initiatives",
    "/schedules",
    "/settings",
];

/// Assert a response is the app shell: the React mount point, and references to
/// the assets the binary embeds.
///
/// This inverts what the old assertion checked. Four self-contained documents
/// had to reference NO external asset — that was the premise `include_str!` of a
/// whole page rested on. One client-side-routed app must reference exactly the
/// assets the binary serves, and referencing one it does not serve is the
/// failure mode worth catching: the page would load, then 404 and render blank.
fn assert_app_shell(path: &str, page: &str) {
    assert!(
        page.contains("id=\"root\""),
        "{path} is not the web build — no React mount point in the served document"
    );
    for asset in ["/assets/app.js", "/assets/vendor.js", "/assets/app.css"] {
        assert!(
            page.contains(asset),
            "{path} does not reference {asset} — the shell is inert without it"
        );
    }
    // Everything it references must be same-origin and embedded. A CDN or a
    // hashed filename would 404 against a binary that embeds fixed paths.
    assert!(
        !page.contains("http://") && !page.contains("https://"),
        "{path} references an absolute URL — every asset must be same-origin and \
         embedded in the binary"
    );
}

/// The `#a=` answer-link view ships with the markdown renderer.
///
/// It was the one surface the SPA-wide markdown rendering missed, so an outside
/// expert saw `## Frage` and `| Option | Risiko |` as literal source while every
/// internal reader saw them rendered. That reader has the *least* context: a
/// `tka_` grant shows one question and nothing else.
///
/// This used to find `renderAnswerPage` in the served bytes and check for an
/// `mdNode` call inside it. A bundled page cannot be sliced that way — and the
/// old comment's complaint, "there is no JS test lane here", no longer holds:
/// `web/src/pages/board/AnswerGrantPage.test.tsx` renders the component and
/// asserts the body becomes ELEMENTS and that its source spelling appears
/// nowhere. What is still this layer's job, and is checked here, is that the
/// page the binary serves carries the grant path and the renderer at all.
#[tokio::test]
async fn answer_link_page_ships_the_grant_view_and_the_renderer() {
    let app = TestApp::spawn().await;
    let body = app.eager_bundle().await;
    assert!(
        body.contains("/answer/self"),
        "/board must carry the `#a=` grant view, which reads and writes /v1/answer/self"
    );
    assert!(
        body.contains("md-table"),
        "the markdown renderer must be in the bundle the answer view renders through"
    );
}

/// The app's assets are served, unauthenticated, with the right content types.
///
/// This is new surface. Four self-contained documents needed no asset routes at
/// all; one client-side-routed app is inert without them, and a wrong
/// `Content-Type` on the JS means the browser refuses to execute it — a blank
/// page with a console error, which no other test would catch.
#[tokio::test]
async fn app_assets_are_served_with_correct_types() {
    let app = TestApp::spawn().await;
    for (path, ct) in [
        ("/assets/app.js", "text/javascript"),
        ("/assets/vendor.js", "text/javascript"),
        ("/assets/runtime.js", "text/javascript"),
        ("/assets/app.css", "text/css"),
    ] {
        let resp = app.request(Method::GET, path).send().await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "{path} should serve");
        let got = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        assert!(
            got.contains(ct),
            "{path} must be served as {ct}, got '{got}' — a browser refuses to \
             execute a script served under the wrong type"
        );
        assert!(
            !resp.text().await.unwrap().is_empty(),
            "{path} served an empty body"
        );
    }
}

/// An asset name that is not embedded 404s, and cannot escape the manifest.
///
/// `/assets/{file}` replaced four routes named one by one, which is what let the
/// editor route be code-split — the bundler names those chunks, so no fixed list
/// could have covered them. The four names were also, incidentally, what made
/// path traversal impossible, and that property has to be shown to survive the
/// change rather than assumed.
///
/// It does survive, and for a stronger reason than sanitizing would give: the
/// path is matched against a compile-time table and never joined onto a
/// directory, so `..` is not a dangerous input, it is simply a name that is not
/// in the table.
#[tokio::test]
async fn unknown_asset_names_404_and_cannot_traverse() {
    let app = TestApp::spawn().await;
    for path in [
        "/assets/nope.js",
        "/assets/..%2f..%2fCargo.toml",
        "/assets/%2e%2e%2f%2e%2e%2fsrc%2fmain.rs",
        "/assets/.env",
    ] {
        let resp = app.request(Method::GET, path).send().await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "{path} must 404 — the manifest is an exact lookup, not a directory"
        );
        let body = resp.text().await.unwrap();
        assert!(
            !body.contains("[package]") && !body.contains("fn main"),
            "{path} served repository content: {body}"
        );
    }
}

/// Assets revalidate with an ETag instead of being cached forever.
///
/// The filenames are deliberately stable (the binary embeds them by
/// name), so nothing in the URL changes between builds. That rules out
/// `immutable` caching and makes the ETag the only thing standing between a
/// deploy and a browser serving last build's JavaScript out of cache — with
/// this build's HTML.
#[tokio::test]
async fn app_assets_revalidate_by_etag() {
    let app = TestApp::spawn().await;
    let resp = app
        .request(Method::GET, "/assets/app.js")
        .send()
        .await
        .unwrap();
    let etag = resp
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("assets must carry an ETag — the filename cannot signal a change")
        .to_string();
    let cache = resp
        .headers()
        .get(reqwest::header::CACHE_CONTROL)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        cache.contains("must-revalidate"),
        "stable filenames must be revalidated, not cached blind, got '{cache}'"
    );
    assert!(
        !cache.contains("immutable"),
        "`immutable` is only safe when the URL changes with the content, and these \
         URLs deliberately do not: '{cache}'"
    );

    // The whole point: a matching validator costs a round trip and no body.
    let again = app
        .request(Method::GET, "/assets/app.js")
        .header(reqwest::header::IF_NONE_MATCH, &etag)
        .send()
        .await
        .unwrap();
    assert_eq!(
        again.status(),
        StatusCode::NOT_MODIFIED,
        "a matching If-None-Match must answer 304, or every navigation re-downloads \
         the whole bundle"
    );
    assert!(
        again.text().await.unwrap().is_empty(),
        "a 304 must carry no body"
    );

    // A stale validator must serve the new bytes rather than a spurious 304.
    let changed = app
        .request(Method::GET, "/assets/app.js")
        .header(reqwest::header::IF_NONE_MATCH, "\"stale\"")
        .send()
        .await
        .unwrap();
    assert_eq!(changed.status(), StatusCode::OK);
    assert!(!changed.text().await.unwrap().is_empty());
}

/// A WEAK validator must still revalidate.
///
/// This is the one that got away. The strong-ETag test above passes locally and
/// passed in CI, and the feature was still broken in production: a compressing
/// proxy (Cloudflare, in front of Render) rewrites the strong `"abc"` this
/// server emits to a weak `W/"abc"`, because compression changes the bytes. The
/// browser returns the weak form, a literal comparison never matches, and every
/// single load re-downloaded the whole vendor bundle.
///
/// `If-None-Match` is specified to use WEAK comparison (RFC 9110 §13.1.2), so
/// the original implementation was simply non-compliant — it just could not be
/// observed without a proxy in the way.
#[tokio::test]
async fn a_weak_validator_still_revalidates() {
    let app = TestApp::spawn().await;
    let strong = app
        .request(Method::GET, "/assets/vendor.js")
        .send()
        .await
        .unwrap()
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("an ETag")
        .to_string();
    assert!(
        !strong.starts_with("W/"),
        "this server emits a STRONG ETag; the weak form is what a proxy makes of it"
    );

    // Exactly what a browser sends back from behind a compressing CDN.
    let weakened = format!("W/{strong}");
    let resp = app
        .request(Method::GET, "/assets/vendor.js")
        .header(reqwest::header::IF_NONE_MATCH, &weakened)
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_MODIFIED,
        "a weak validator for the same body must answer 304 — anything else means \
         every load behind a CDN re-downloads the whole bundle"
    );

    // And a weak validator for DIFFERENT content must still miss.
    let other = app
        .request(Method::GET, "/assets/app.js")
        .header(reqwest::header::IF_NONE_MATCH, &weakened)
        .send()
        .await
        .unwrap();
    assert_eq!(
        other.status(),
        StatusCode::OK,
        "vendor.js's validator must not satisfy a request for app.js"
    );
}

/// No inline script means the CSP no longer has to allow one.
///
/// This is the security dividend of the move to one bundle, and it is worth a
/// test because it is easy to lose: someone adds one inline `<script>` for a
/// quick fix, the CSP blocks it, and the tempting repair is to put
/// `'unsafe-inline'` back — which silently re-opens the whole class of injected
/// script attacks against a page holding a bearer token in localStorage.
#[tokio::test]
async fn script_src_allows_no_inline_script() {
    let app = TestApp::spawn().await;
    let resp = app.request(Method::GET, "/board").send().await.unwrap();
    let csp = resp
        .headers()
        .get("content-security-policy")
        .and_then(|v| v.to_str().ok())
        .expect("a page route must send a CSP")
        .to_string();

    let script_src = csp
        .split(';')
        .map(str::trim)
        .find(|d| d.starts_with("script-src"))
        .expect("the CSP must name script-src explicitly, not fall back to default-src");
    assert!(
        !script_src.contains("unsafe-inline"),
        "script-src must not allow inline script — the app is bundled, so nothing \
         needs it: '{script_src}'"
    );
    assert!(
        script_src.contains("'self'"),
        "script-src must allow the app's own same-origin bundle: '{script_src}'"
    );

    // …and the document must actually honour that: an inline <script> would be
    // blocked at runtime, so its presence means a blank page in a real browser.
    let page = resp.text().await.unwrap();
    for open in ["<script>", "<script type=\"module\">"] {
        assert!(
            !page.contains(open),
            "the served document carries an inline script ({open}), which this CSP blocks"
        );
    }
}

/// The display face is embedded and served from `/assets/`, so the CSP must
/// allow a same-origin font — and must say so in its own directive rather than
/// leaning on `default-src`, which a later edit could narrow without noticing
/// that it had just stopped the headings from loading.
#[tokio::test]
async fn font_src_allows_the_embedded_display_face() {
    let app = TestApp::spawn().await;
    let resp = app.request(Method::GET, "/board").send().await.unwrap();
    let csp = resp
        .headers()
        .get("content-security-policy")
        .and_then(|v| v.to_str().ok())
        .expect("a page route must send a CSP")
        .to_string();

    let font_src = csp
        .split(';')
        .map(str::trim)
        .find(|d| d.starts_with("font-src"))
        .expect("the CSP must name font-src explicitly, not fall back to default-src");
    assert!(
        font_src.contains("'self'"),
        "font-src must allow the embedded same-origin face: '{font_src}'"
    );
    assert!(
        !font_src.contains("http"),
        "the face is embedded in the binary; font-src must name no external host: '{font_src}'"
    );

    // And the face must actually be there to serve. `build.rs` embeds whatever
    // `web/dist/assets/` holds, so a bundle built without the font would pass
    // every check above and still render the fallback stack.
    let woff2 = app
        .request(Method::GET, "/assets/geist-latin-wght-normal.woff2")
        .send()
        .await
        .unwrap();
    assert_eq!(
        woff2.status(),
        StatusCode::OK,
        "the display face must be embedded and served"
    );
    assert_eq!(
        woff2
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("font/woff2"),
    );
}

#[tokio::test]
async fn favicon_served_unauthenticated_as_svg() {
    let app = TestApp::spawn().await;
    for path in ["/favicon.svg", "/favicon.ico"] {
        let resp = app.request(Method::GET, path).send().await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "{path} should serve");
        let ct = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        assert!(
            ct.contains("image/svg+xml"),
            "{path} should be served as SVG, got '{ct}'"
        );
        let body = resp.text().await.unwrap();
        assert!(body.contains("<svg"), "{path} body should be an SVG");
    }
}

#[tokio::test]
async fn workflow_enforcement_illegal_transition_teaches() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Illegal transition test").await;

    let (status, body) = app.transition(&app.admin, &id, "done").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "transition.illegal");
    assert_eq!(body["current_state"], "brief");
    let allowed: Vec<&str> = body["allowed_transitions"]
        .as_array()
        .expect("allowed_transitions present")
        .iter()
        .map(|t| t["to"].as_str().unwrap())
        .collect();
    assert!(
        allowed.contains(&"spec") && allowed.contains(&"cancelled"),
        "{allowed:?}"
    );
    assert!(body["remedy"].as_str().unwrap().contains("/transition"));
    assert!(body["message"].as_str().unwrap().contains("brief"));

    // Unknown state also teaches.
    let (status, body) = app.transition(&app.admin, &id, "nonexistent").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "transition.unknown_state");
    assert!(body["allowed_transitions"].is_array());
}

#[tokio::test]
async fn scope_gate_403_without_human() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Scope gate test").await;
    let (s, b) = app.transition(&app.worker, &id, "spec").await;
    assert_eq!(s, StatusCode::OK, "{b}");

    // spec -> ready requires scope:human; the worker lacks it.
    let (status, body) = app.transition(&app.worker, &id, "ready").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"], "transition.scope");
    assert!(body["message"].as_str().unwrap().contains("human"));
    assert!(body["allowed_transitions"].is_array());

    // With the human scope it passes.
    let (status, body) = app.transition(&app.human, &id, "ready").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "ready");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_ready_claims_are_exactly_once() {
    let app = TestApp::spawn().await;
    const N: usize = 8;
    for i in 0..N {
        let id = app
            .create_ticket(&format!("Concurrent claim target number {i}"))
            .await;
        app.to_ready(&id).await;
    }

    // 16 simultaneous claimers race for 8 tickets: exactly 8 win distinct
    // tickets, the rest get 204.
    let mut futures = Vec::new();
    for i in 0..(N * 2) {
        let token = if i % 2 == 0 {
            app.worker.clone()
        } else {
            app.worker2.clone()
        };
        let client = app.client.clone();
        let base = app.base.clone();
        futures.push(tokio::spawn(async move {
            let resp = client
                .post(format!("{base}/v1/ready/claim"))
                .bearer_auth(token)
                .json(&json!({ "project": "tp" }))
                .send()
                .await
                .expect("claim request");
            let status = resp.status();
            let body = resp.json::<Value>().await.unwrap_or(Value::Null);
            (status, body)
        }));
    }
    let results: Vec<(StatusCode, Value)> = join_all(futures)
        .await
        .into_iter()
        .map(|r| r.expect("join"))
        .collect();

    let mut claimed_ids: Vec<String> = results
        .iter()
        .filter(|(s, _)| *s == StatusCode::OK)
        .map(|(_, b)| b["id"].as_str().expect("claimed id").to_string())
        .collect();
    let misses = results
        .iter()
        .filter(|(s, _)| *s == StatusCode::NO_CONTENT)
        .count();

    assert_eq!(claimed_ids.len(), N, "exactly {N} claims must succeed");
    assert_eq!(misses, N, "the other {N} callers must get 204");
    claimed_ids.sort();
    claimed_ids.dedup();
    assert_eq!(
        claimed_ids.len(),
        N,
        "no ticket may be handed to two claimants"
    );

    // Every winner got a lease with a fence.
    for (s, b) in &results {
        if *s == StatusCode::OK {
            assert!(b["lease"]["fence"].as_i64().unwrap() >= 1);
            assert!(b["claim"]["holder"].is_string());
        }
    }
}

/// Reads must not serialize writers. `GET /v1/export` with no project filter
/// walks the whole tickets table and, per ticket, queries its deps and its
/// comments — the longest read in the API. While it is in flight, claims and
/// heartbeats have to keep completing at their normal speed.
///
/// Before the read connections landed, `with_conn` and `with_tx` shared one
/// `Mutex<Connection>`, so one export stalled every claim, transition and
/// heartbeat in the process for as long as the scan took. Measured on this test
/// at 8k tickets: the worst claim during an export was **104ms** (~80% of the
/// whole export) against a 1.8ms worst case with the store idle. With the read
/// connections: **6-17ms**, and roughly twice as many claims complete during the
/// same export.
///
/// The claim is issued in a loop rather than once, because a single sample can
/// miss the scan window and pass a broken store. The loop samples the entire
/// export: the first iteration claims, the rest are lease renewals — both are
/// `with_tx` writes, which is exactly what used to queue.
///
/// And the export is run `ROUNDS` times rather than once, because that is what
/// makes the verdict robust (takomo-hcv7). Serialization is *systematic* — every
/// export blocks a claim, so every round is bad — while a scheduling stall on a
/// busy machine is *sporadic*, corrupting one round. Taking the median across
/// rounds separates the two by construction. The reasoning behind each assertion,
/// and the numbers both cases produce, are at the assertions themselves.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn long_export_does_not_stall_claims_and_heartbeats() {
    let app = TestApp::spawn().await;
    let target = app
        .create_ticket("Claim me while a full export is in flight")
        .await;
    app.to_ready(&target).await;
    // Enough rows that an unfiltered export is a real scan, seeded straight into
    // the DB — 8k tickets over HTTP would dominate the suite's runtime.
    const BULK: usize = 8_000;
    app.seed_bulk_tickets(BULK);

    // Warm the page cache, so the timings below measure the scan and not the
    // first read of a cold file.
    let (warm, _, _) = app.get_raw(&app.admin, "/v1/export").await;
    assert_eq!(warm, StatusCode::OK);

    // What a claim/renewal costs with the store otherwise idle. This is the
    // baseline the during-export samples are compared against, so take enough of
    // them that its median is a median and not a coin flip.
    let mut idle: Vec<Duration> = Vec::new();
    for _ in 0..200 {
        let started = Instant::now();
        let (s, lease) = app
            .post(
                &app.worker,
                &format!("/v1/tickets/{target}/claim"),
                json!({}),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "claim failed: {lease}");
        assert!(lease["fence"].as_i64().unwrap() >= 1);
        idle.push(started.elapsed());
    }
    idle.sort();
    let idle_median = idle[idle.len() / 2];
    let idle_worst = *idle.last().expect("idle samples");

    // What the export costs with nothing to contend with.
    let started = Instant::now();
    let (status, _, text) = app.get_raw(&app.admin, "/v1/export").await;
    let solo_export = started.elapsed();
    assert_eq!(status, StatusCode::OK);
    assert!(
        text.lines().count() > BULK,
        "export must cover the bulk rows, got {} lines",
        text.lines().count()
    );
    assert!(
        solo_export >= Duration::from_millis(50),
        "the export ran in {solo_export:?} — too fast to prove anything; raise BULK"
    );

    // Now the same export, sampled by claims for its whole duration — ROUNDS
    // times, so the verdict rests on how the store behaves every time an export
    // runs rather than on one window's luck.
    const ROUNDS: usize = 5;
    let mut round_worst: Vec<Duration> = Vec::new();
    let mut round_throughput: Vec<u128> = Vec::new();
    for round in 0..ROUNDS {
        let client = app.client.clone();
        let url = app.url("/v1/export");
        let admin = app.admin.clone();
        let export = tokio::spawn(async move {
            let resp = client
                .get(url)
                .bearer_auth(admin)
                .send()
                .await
                .expect("export request");
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            (status, body.lines().count())
        });

        let started_round = Instant::now();
        let mut during: Vec<Duration> = Vec::new();
        while !export.is_finished() {
            let started = Instant::now();
            let (s, lease) = app
                .post(
                    &app.worker,
                    &format!("/v1/tickets/{target}/claim"),
                    json!({}),
                )
                .await;
            assert_eq!(s, StatusCode::OK, "claim during export failed: {lease}");
            during.push(started.elapsed());
        }
        let wall = started_round.elapsed();
        let (export_status, export_lines) = export.await.expect("join export task");
        assert_eq!(export_status, StatusCode::OK);
        assert!(
            export_lines > BULK,
            "the concurrent export must be complete"
        );
        assert!(
            !during.is_empty(),
            "round {round} took no claim samples during a {wall:?} export"
        );

        // The CONTROL, taken immediately after the export finishes and in the
        // same round: the same claim, the same loop, the same machine, with no
        // read in flight. Same count as the during-samples so the two medians
        // are drawn from comparable populations.
        //
        // This is the whole point of the round's shape. Every statistic that
        // compares the during-samples against something measured in a DIFFERENT
        // phase — an idle baseline taken minutes earlier, or the round's own
        // wall clock — silently measures how loaded the machine was, and on a
        // busy runner that is indistinguishable from the store serializing.
        let mut control: Vec<Duration> = Vec::with_capacity(during.len());
        for _ in 0..during.len() {
            let started = Instant::now();
            let (s, lease) = app
                .post(
                    &app.worker,
                    &format!("/v1/tickets/{target}/claim"),
                    json!({}),
                )
                .await;
            assert_eq!(s, StatusCode::OK, "control claim failed: {lease}");
            control.push(started.elapsed());
        }

        during.sort();
        control.sort();
        let worst = *during.last().expect("samples during the export");
        let median = during[during.len() / 2];
        let control_median = control[control.len() / 2];
        // How much slower a typical claim is while a read is in flight. Scaled
        // by 100 so it stays integer arithmetic: 100 means identical.
        //
        // Serialization that is spread thinly — every claim waiting a little,
        // rather than one claim absorbing the whole lock hold — moves the two
        // medians apart and shows up here. CPU starvation moves them together,
        // because both halves of the round queue for the same cores, so it does
        // not.
        let slowdown = median.as_nanos() * 100 / control_median.as_nanos().max(1);
        eprintln!(
            "round {round}: export {wall:?} | {} claims, median {median:?} vs control \
             {control_median:?} ({slowdown}%), worst {worst:?}",
            during.len(),
        );
        round_worst.push(worst);
        round_throughput.push(slowdown);
    }

    round_worst.sort();
    round_throughput.sort();
    let typical_worst = round_worst[ROUNDS / 2];
    let typical_slowdown = round_throughput[ROUNDS / 2];
    eprintln!(
        "export {solo_export:?} solo | idle claim median {idle_median:?} worst \
         {idle_worst:?} | across {ROUNDS} rounds: typical worst claim \
         {typical_worst:?}, typical claim {typical_slowdown}% of its same-round \
         control"
    );

    // ---------------------------------------------------------------------
    // takomo-hcv7. The old assertion was
    //
    //     worst * 4 < solo_export || worst < Duration::from_millis(25)
    //
    // over a single export. It guaranteed: no one claim took more than a quarter
    // of a solo export, unless it came in under 25ms outright. That is the right
    // *property* — the signal really does live in the tail, because a store that
    // serializes gives the export one lock hold and exactly one claim eats it —
    // but `worst` over one window is the wrong *statistic*. A scheduling stall on
    // a busy machine forges a single large sample with the same shape as the
    // signal: the CI red on PR #87 was a 79.9ms sample against a 128ms export,
    // 62% of it, which is indistinguishable from genuine serialization. No
    // threshold on one sample can tell those apart, and the 25ms floor was too
    // tight to rescue it. It failed twice on 2026-07-28 with three orders of
    // magnitude of real headroom, once in CI on a PR touching no read path, and
    // blocked a merge.
    //
    // What separates them is not magnitude but *repetition*. Serialization is
    // systematic: every export blocks a claim, so every round is bad. A stall is
    // sporadic: it corrupts one round. So keep the assertion and take the median
    // of the per-round worst instead of one window's worst. Measured here:
    //
    //                              typical worst   throughput   verdict
    //   read connections (today)         ~0.6ms          ~95%   pass
    //   with_conn on the writer          ~98ms           ~21%   FAIL
    //
    // and a single 80ms stall in one of five rounds leaves the median at ~0.6ms.
    //
    // The second assertion exists because the first one's statistic is a tail:
    // it catches serialization that gives one claim the whole lock hold, and
    // would miss it if it were spread thinly enough that no single claim stood
    // out. So the second compares a typical claim DURING the export against a
    // typical claim measured moments later, in the same round, with no export
    // running.
    //
    // That control is the fix for the second flakiness round. The statistic it
    // replaces was the share of the export's wall clock spent in typical-cost
    // claims, and it was wrong in a way that only showed under load: the claim
    // loop is sequential, so when the process is starved of CPU it issues fewer
    // claims per second while each one still costs what it always cost. The
    // number that produced then measured how much CPU the TEST LOOP got and
    // reported it as how much the LOCK let writers through. Measured on two
    // contended cores: 650 claims per round at a 212µs median — identical to the
    // idle median, so writers were plainly not blocked — and the statistic read
    // 21%, which is the same number this file documents for a genuinely
    // serializing store. A healthy store and a broken one were indistinguishable
    // on it, and the failure message accused the code of a bug it did not have.
    //
    // Comparing against the idle baseline taken earlier in the test has the same
    // defect for the same reason: it depends on how loaded the machine was
    // during a *different* phase. The control has to be measured in the same
    // round, through the same loop, on the same cores. Then CPU starvation moves
    // both halves together and cancels, while serialization moves them apart.
    //
    // Measured three ways, with `readers` forced empty to reproduce the
    // pre-read-connections store:
    //
    //                                    typical worst   during/control   result
    //   healthy, idle                          ~1.5ms            ~112%    pass
    //   healthy, 2 cores + 4 busy loops         ~17ms         96%-120%    pass
    //   with_conn on the writer                ~150ms            ~115%    FAIL
    //
    // Read the last row carefully, because it says something worth knowing: the
    // regression this test exists for is caught ENTIRELY by the first assertion.
    // The single-writer mutex gives one claim per export the whole lock hold —
    // 150ms of a 186ms export — while every other claim is untouched, so the
    // median lands at 300µs against a 317µs control. The control ratio does not
    // move for it, and is not meant to.
    //
    // What the control ratio is for is the case the first assertion cannot see:
    // serialization spread thinly enough that no single claim stands out. No
    // implementation here has produced that, so it guards a shape rather than a
    // known bug — kept because it is nearly free and, unlike the statistic it
    // replaces, it cannot fire when the machine is merely busy. If it ever does
    // fire, the median claim really did get slower while a read was in flight.
    //
    // The 4x threshold is deliberately loose. A typical claim is ~200µs, so a
    // few hundred microseconds of scheduling noise on either side moves this
    // ratio a long way, while diffuse serialization against a >100ms export
    // would move it by an order of magnitude.
    // ---------------------------------------------------------------------
    assert!(
        typical_worst * 4 < solo_export,
        "across {ROUNDS} rounds the typical worst claim during an export was \
         {typical_worst:?}, out of an export that runs in {solo_export:?} alone \
         (idle worst: {idle_worst:?}) — reads are serializing writers again. \
         Per-round worsts: {round_worst:?}"
    );
    assert!(
        typical_slowdown < 400,
        "across {ROUNDS} rounds a typical claim cost {typical_slowdown}% of what the \
         same claim cost moments later with no export running — reads are slowing \
         writers down across the board, i.e. serializing them again. Per-round: \
         {round_throughput:?}"
    );
}

#[tokio::test]
async fn fence_goes_stale_after_expiry_and_reclaim() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Fence staleness test").await;
    app.to_ready(&id).await;

    // Worker 1 claims with a 1-second lease.
    let old_fence = app.claim_ttl(&app.worker, &id, Some(1)).await;

    // Let the lease expire (sweep interval is 250ms in tests).
    tokio::time::sleep(Duration::from_millis(1600)).await;

    // The lease_expired event was emitted and the ticket is ready again.
    let (_, events) = app
        .get(
            &app.admin,
            &format!("/v1/events?since=0&ticket={id}&kind=lease_expired"),
        )
        .await;
    assert_eq!(
        events["events"].as_array().unwrap().len(),
        1,
        "lease_expired event expected: {events}"
    );

    // Worker 2 claims it; the fence must be strictly greater.
    let new_fence = app.claim_as(&app.worker2, &id).await;
    assert!(new_fence > old_fence);

    // Zombie worker 1 heartbeats with the stale fence: teaching 409.
    let (s, body) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/heartbeat"),
            json!({ "fence": old_fence }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(body["code"], "fence.stale");

    // Stale-fence patch by the zombie also bounces (claim held by w2).
    let (s, body) = app
        .patch(
            &app.worker,
            &format!("/v1/tickets/{id}"),
            json!({ "title": "hijack attempt", "fence": old_fence }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(body["code"], "claim.held");
}

/// Drive `id` to `implementing` on a one-second lease and let the lease lapse:
/// the exact position takomo-jb5i is about, reached the way an agent reaches it.
/// Returns the fence of the lease that expired.
///
/// The sleep is 1600ms against a 250ms sweeper, so the claim row is really gone
/// by the time this returns — the state a lapsed holder is normally found in.
async fn lapse_mid_implementation(app: &TestApp, id: &str) -> i64 {
    app.to_ready(id).await;
    let fence = app.claim_ttl(&app.worker, id, Some(1)).await;
    let (s, b) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/transition"),
            json!({ "to": "implementing", "fence": fence }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "->implementing failed: {b}");
    tokio::time::sleep(Duration::from_millis(1600)).await;
    fence
}

#[tokio::test]
async fn lapsed_holder_resumes_the_lease_in_place_instead_of_deadlocking() {
    // takomo-jb5i, the substance behind the error wording: an agent whose lease
    // expired while the work ran could not finish the ticket at all. `done` (here
    // `review`) requires a claim, and `claim` was refused because `implementing`
    // is not claimable — the two errors pointed at each other, and the only escape
    // walked the ticket backwards through the ready queue, where another worker
    // could take it.
    //
    // The claim is now honoured in a non-claimable state when — and only when —
    // the caller is the holder whose own lease lapsed there and nobody has claimed
    // since. Fencing is untouched: the resume bumps the fence like any claim.
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Close after the lease expired").await;
    let old_fence = lapse_mid_implementation(&app, &id).await;

    // The deadlock's first half is unchanged: no lease, no claim-gated move.
    let (s, err) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/transition"),
            json!({ "to": "review" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "expected a claim complaint: {err}");
    assert_eq!(err["code"], "transition.claim_required");

    // But the remedy now leads with the claim, and says why that works here.
    let remedy = err["remedy"].as_str().expect("remedy");
    assert!(
        remedy.starts_with(&format!("POST /v1/tickets/{id}/claim")),
        "the remedy must lead with the call that now works: {remedy}"
    );
    assert!(
        remedy.contains("resumed") && remedy.contains("does NOT go back to the ready queue"),
        "the remedy must say it is a resume in place: {remedy}"
    );
    assert_eq!(
        err["details"]["resume_in_place"],
        json!(true),
        "machine readers get the same verdict: {err}"
    );

    // And it is honoured: a claim in a state the workflow does not mark claimable.
    let (s, lease) = app
        .post(&app.worker, &format!("/v1/tickets/{id}/claim"), json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "the resume must be granted: {lease}");
    assert_eq!(
        lease["resumed"],
        json!(true),
        "flagged as a resume: {lease}"
    );
    let new_fence = lease["fence"].as_i64().expect("fence");
    assert!(
        new_fence > old_fence,
        "a resume is still a claim, so the fence moves: {lease}"
    );

    // The transition the agent came to make now goes through.
    let (s, b) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/transition"),
            json!({ "to": "review", "fence": new_fence }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "->review failed: {b}");
    assert_eq!(b["state"], "review");

    // The old fence is dead all the same — the resume superseded it.
    let (s, stale) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/heartbeat"),
            json!({ "fence": old_fence }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "stale fence must bounce: {stale}");
    assert_eq!(stale["code"], "fence.stale");

    // The log tells the story, and the ticket never re-entered the ready queue —
    // the cost the old remedy warned about is simply not paid.
    let (_, ev) = app
        .get(&app.admin, &format!("/v1/events?since=0&ticket={id}"))
        .await;
    let events = ev["events"].as_array().expect("events");
    let claimed: Vec<&Value> = events.iter().filter(|e| e["kind"] == "claimed").collect();
    assert_eq!(claimed.len(), 2, "the original claim and the resume: {ev}");
    assert_eq!(
        claimed[1]["payload"]["resumed_after_expiry"],
        json!(true),
        "a supervisor can count resumes: {ev}"
    );
    // Exactly one entry into `ready`: the spec approval during setup. A second one
    // would be the trip back through the queue the old remedy had to prescribe.
    let into_ready = events
        .iter()
        .filter(|e| e["kind"] == "transitioned" && e["payload"]["to"] == "ready")
        .count();
    assert_eq!(
        into_ready, 1,
        "the ticket must not have gone back through the queue: {ev}"
    );
}

#[tokio::test]
async fn a_lapsed_lease_is_resumable_only_by_its_own_holder() {
    // The other half of the safety argument, and the reason fencing is not
    // weakened: the resume is granted on the strength of *who* lost the lease. A
    // second worker gets exactly the old refusals — `claim.state` on the claim, and
    // the re-entry route on the transition — plus the name of the actor whose
    // lapsed lease it is, so it stops retrying.
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Not yours to resume").await;
    lapse_mid_implementation(&app, &id).await;

    let (s, refused) = app
        .post(&app.worker2, &format!("/v1/tickets/{id}/claim"), json!({}))
        .await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "an interloper must not resume: {refused}"
    );
    assert_eq!(refused["code"], "claim.state");
    assert_eq!(refused["details"]["lapsed_holder"], "agent:w1");
    assert!(
        refused["message"]
            .as_str()
            .expect("message")
            .contains("agent:w1"),
        "say whose lease it was: {refused}"
    );

    // Its transition attempt gets the re-entry route — the takomo-jb5i remedy
    // still applies to everyone who is not the lapsed holder.
    let (s, err) = app
        .post(
            &app.worker2,
            &format!("/v1/tickets/{id}/transition"),
            json!({ "to": "review" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "expected a claim complaint: {err}");
    assert_eq!(err["code"], "transition.claim_required");
    assert_eq!(err["details"]["resume_in_place"], json!(false));
    assert_eq!(err["details"]["lapsed_holder"], "agent:w1");
    let message = err["message"].as_str().expect("message");
    let remedy = err["remedy"].as_str().expect("remedy");
    assert!(
        message.contains("not claimable") && message.contains("claim.state"),
        "the message must warn that claiming is refused for this caller: {message}"
    );
    assert!(
        !remedy.starts_with(&format!("POST /v1/tickets/{id}/claim")),
        "the remedy must not lead with a call that is refused: {remedy}"
    );
    assert!(
        remedy.contains("\"to\":\"ready\""),
        "the remedy must name the re-entry state: {remedy}"
    );
    assert_eq!(
        err["details"]["reentry_states"],
        json!(["ready"]),
        "machine readers get the route too: {err}"
    );
    assert_eq!(err["details"]["claimable_states"], json!(["spec", "ready"]));

    // And walking it still works: whoever really is starting over goes through the
    // queue, which is exactly the trip the lapsed holder no longer has to make.
    let (s, b) = app.transition(&app.worker2, &id, "ready").await;
    assert_eq!(s, StatusCode::OK, "re-entry to ready failed: {b}");
    let fence = app.claim_as(&app.worker2, &id).await;
    for to in ["implementing", "review"] {
        let (s, b) = app
            .post(
                &app.worker2,
                &format!("/v1/tickets/{id}/transition"),
                json!({ "to": to, "fence": fence }),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "->{to} failed: {b}");
    }

    // Now that someone else has claimed it, the original holder's resume is gone
    // for good — even once the ticket is unclaimed again. The successor's claim
    // bumped the fence and took the marker with it, and letting go on purpose is
    // not a lapse, so there is nothing left for the first worker to resume.
    let (s, released) = app
        .post(
            &app.worker2,
            &format!("/v1/tickets/{id}/release"),
            json!({ "fence": fence }),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "release failed: {released}");
    let (s, gone) = app
        .post(&app.worker, &format!("/v1/tickets/{id}/claim"), json!({}))
        .await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "a superseded holder never resumes: {gone}"
    );
    assert_eq!(gone["code"], "claim.state");
    assert_eq!(gone["details"]["lapsed_holder"], Value::Null);
}

#[tokio::test]
async fn lapsed_lease_resumes_before_the_sweeper_notices() {
    // Expiry is noticed twice — lazily by the next write on the ticket, and by the
    // sweeper — so the evidence a resume rests on lives in two places: the still
    // recorded expired claim, and the marker left once it is cleared. With no
    // sweeper running only the first exists, and the resume has to work anyway;
    // otherwise the outcome would depend on whether a 250ms timer had fired.
    let app = TestApp::spawn_without_sweeper().await;
    let id = app.create_ticket("Resume before the sweep").await;
    app.to_ready(&id).await;
    let fence = app.claim_ttl(&app.worker, &id, Some(1)).await;
    let (s, b) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/transition"),
            json!({ "to": "implementing", "fence": fence }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "->implementing failed: {b}");
    tokio::time::sleep(Duration::from_millis(1100)).await;

    let (s, lease) = app
        .post(&app.worker, &format!("/v1/tickets/{id}/claim"), json!({}))
        .await;
    assert_eq!(
        s,
        StatusCode::OK,
        "resume with the claim still on the row: {lease}"
    );
    assert_eq!(lease["resumed"], json!(true));
    let (s, b) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/transition"),
            json!({ "to": "review", "fence": lease["fence"] }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "->review failed: {b}");
}

#[tokio::test]
async fn lapsed_holder_closes_on_the_simple_workflow() {
    // Where the bug was actually met. `simple` is what `takomo init` applies, so it
    // is the workflow most installs run: `in_progress` is not claimable and
    // `in_progress -> done` requires a claim, which is the deadlock in its purest
    // form — the ticket is finished and nothing can say so.
    let app = TestApp::spawn().await;
    app.create_project_with("sw", common::simple_workflow())
        .await;
    let id = app.create_ticket_in("sw", "Ship it").await;

    let fence = app.claim_ttl(&app.worker, &id, Some(1)).await;
    let (s, b) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/transition"),
            json!({ "to": "in_progress", "fence": fence }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "->in_progress failed: {b}");
    tokio::time::sleep(Duration::from_millis(1600)).await;

    let (s, lease) = app
        .post(&app.worker, &format!("/v1/tickets/{id}/claim"), json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "resume on `simple`: {lease}");
    let (s, b) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/transition"),
            json!({ "to": "done", "fence": lease["fence"] }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "->done failed: {b}");
    assert_eq!(b["state"], "done");
}

#[tokio::test]
async fn claim_required_from_a_claimable_state_still_points_at_claim() {
    // The other half of takomo-jb5i: where claiming *does* work, the remedy must
    // still be the plain claim — the fix must not send everyone on a detour.
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Claim me first").await;
    app.to_ready(&id).await;

    let (s, err) = app.transition(&app.worker, &id, "implementing").await;
    assert_eq!(s, StatusCode::CONFLICT, "expected a claim complaint: {err}");
    assert_eq!(err["code"], "transition.claim_required");
    let remedy = err["remedy"].as_str().expect("remedy");
    assert!(
        remedy.starts_with(&format!("POST /v1/tickets/{id}/claim")),
        "ready is claimable, so claiming is the remedy: {remedy}"
    );
    assert!(
        remedy.contains("\"to\":\"implementing\""),
        "the remedy names the transition being retried: {remedy}"
    );
}

#[tokio::test]
async fn heartbeat_renewal_emits_no_event() {
    // ts-8zks: lease renewal is silent bookkeeping. Heartbeats must not write
    // to the append-only event log (they flood it at fleet scale), while the
    // lease itself is still renewed.
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Heartbeat quiet test").await;
    app.to_ready(&id).await;

    let fence = app.claim_ttl(&app.worker, &id, Some(900)).await;

    // Two heartbeats on the same lease.
    for _ in 0..2 {
        let (s, body) = app
            .post(
                &app.worker,
                &format!("/v1/tickets/{id}/heartbeat"),
                json!({ "fence": fence }),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "heartbeat should renew: {body}");
        assert!(body["expires_at"].is_string(), "lease renewed: {body}");
    }

    // An idempotent re-claim by the same holder is also a renewal.
    app.claim(&id).await;

    // No heartbeat event ever reached the log.
    let (_, hb) = app
        .get(
            &app.admin,
            &format!("/v1/events?since=0&ticket={id}&kind=heartbeat"),
        )
        .await;
    assert_eq!(
        hb["events"].as_array().unwrap().len(),
        0,
        "no heartbeat events expected in the log: {hb}"
    );

    // The claim itself is still observable (exactly one `claimed`).
    let (_, claimed) = app
        .get(
            &app.admin,
            &format!("/v1/events?since=0&ticket={id}&kind=claimed"),
        )
        .await;
    assert_eq!(
        claimed["events"].as_array().unwrap().len(),
        1,
        "the claim is still logged: {claimed}"
    );
}

#[tokio::test]
async fn body_replacement_requires_cas() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("CAS body test").await;

    // No If-Match: refused with instructions.
    let (s, body) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "body": "new body" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(body["code"], "conflict.if_match_required");
    assert_eq!(body["current_version"], 1);

    // Wrong If-Match: version conflict with current version + body hash.
    let (s, body) = app
        .patch_with(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            &[("If-Match", "\"99\"")],
            json!({ "body": "new body" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(body["code"], "conflict.version");
    assert_eq!(body["current_version"], 1);
    assert!(body["details"]["body_sha256"].is_string());

    // Correct If-Match succeeds and bumps the version.
    let (s, body) = app
        .patch_with(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            &[("If-Match", "\"1\"")],
            json!({ "body": "new body" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body["body"], "new body");
    assert_eq!(body["version"], 2);

    // ETag on GET reflects the version.
    let resp = app
        .authed(Method::GET, &app.admin, &format!("/v1/tickets/{id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.headers().get("ETag").unwrap().to_str().unwrap(),
        "\"2\""
    );
}

#[tokio::test]
async fn idempotent_create_replays() {
    let app = TestApp::spawn().await;
    let req = json!({ "project": "tp", "title": "Idempotency replay test" });

    let (s, first) = app
        .post_with(
            &app.admin,
            "/v1/tickets",
            &[("Idempotency-Key", "create-once")],
            req.clone(),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);

    let (s, second) = app
        .post_with(
            &app.admin,
            "/v1/tickets",
            &[("Idempotency-Key", "create-once")],
            req.clone(),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "replay must be 200, not a twin 201");
    assert_eq!(first["id"], second["id"]);

    // Only one ticket exists with that title.
    let (_, list) = app
        .get(&app.admin, "/v1/tickets?project=tp&q=Idempotency+replay")
        .await;
    assert_eq!(list["items"].as_array().unwrap().len(), 1);

    // similar[] hints on a keyword-overlapping create.
    let (status, body) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "tp", "title": "Idempotency replay follow-up work" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let similar = body["similar"].as_array().unwrap();
    assert!(
        similar.iter().any(|s| s["id"] == first["id"]),
        "similar should mention the twin: {body}"
    );
}

#[tokio::test]
async fn idempotent_create_refuses_key_reuse_with_different_body() {
    let app = TestApp::spawn().await;
    let (s, _) = app
        .post_with(
            &app.admin,
            "/v1/tickets",
            &[("Idempotency-Key", "create-body-bound")],
            json!({ "project": "tp", "title": "First body" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);

    let (s, err) = app
        .post_with(
            &app.admin,
            "/v1/tickets",
            &[("Idempotency-Key", "create-body-bound")],
            json!({ "project": "tp", "title": "Different body" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{err}");
    assert_eq!(err["code"], "conflict.idempotency_key");
}

#[tokio::test]
async fn idempotent_comment_replays() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Comment idempotency").await;
    let body = json!({ "body": "verified by this note" });

    let (s, first) = app
        .post_with(
            &app.admin,
            &format!("/v1/tickets/{id}/comments"),
            &[("Idempotency-Key", "comment-once")],
            body.clone(),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{first}");

    let (s, second) = app
        .post_with(
            &app.admin,
            &format!("/v1/tickets/{id}/comments"),
            &[("Idempotency-Key", "comment-once")],
            body,
        )
        .await;
    assert_eq!(
        s,
        StatusCode::OK,
        "replay must be 200, not a twin 201: {second}"
    );
    assert_eq!(first["id"], second["id"]);

    let (_, detail) = app
        .get(&app.admin, &format!("/v1/tickets/{id}?include=comments"))
        .await;
    assert_eq!(detail["comments"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn blocked_tickets_never_ready_including_inherited() {
    let app = TestApp::spawn().await;
    let blocker = app.create_ticket("The blocker nobody finished").await;
    let epic = app.create_ticket("Epic parent blocked by dependency").await;
    let child = app
        .create_ticket("Child inherits the ancestor blockage")
        .await;

    // child under epic; epic blocked_by blocker.
    let (s, _) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{child}"),
            json!({ "parent": epic }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{epic}/deps"),
            json!({ "blocked_by": blocker }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);

    app.to_ready(&epic).await;
    app.to_ready(&child).await;

    // Neither epic (directly blocked) nor child (via ancestor) is ready.
    let (_, ready) = app.get(&app.admin, "/v1/ready?project=tp").await;
    let ids: Vec<&str> = ready["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert!(
        !ids.contains(&epic.as_str()),
        "blocked epic in ready: {ids:?}"
    );
    assert!(
        !ids.contains(&child.as_str()),
        "ancestor-blocked child in ready: {ids:?}"
    );

    // Direct claim also refuses, naming the blocker.
    let (s, body) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{child}/claim"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(body["code"], "claim.blocked");
    assert!(body["message"].as_str().unwrap().contains(&blocker));

    // Terminal blocker unblocks both (cancelled is terminal).
    let (s, b) = app.transition(&app.admin, &blocker, "cancelled").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (_, ready) = app.get(&app.admin, "/v1/ready?project=tp").await;
    let ids: Vec<&str> = ready["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert!(
        ids.contains(&epic.as_str()) && ids.contains(&child.as_str()),
        "{ids:?}"
    );

    // Dependency cycles are refused.
    let (s, body) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{blocker}/deps"),
            json!({ "blocked_by": epic }),
        )
        .await;
    // blocker <- epic already exists, so epic blocked_by blocker + this = cycle
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "validation.dep_cycle");
}

#[tokio::test]
async fn no_open_children_guard_blocks_done() {
    let app = TestApp::spawn().await;
    let parent = app.create_ticket("Parent epic with open child").await;
    let child = app.create_ticket("Open child of the epic").await;
    let (s, _) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{child}"),
            json!({ "parent": parent }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);

    // Drive parent to review: ready -> claim -> implementing -> review -> release.
    app.to_ready(&parent).await;
    let fence = app.claim(&parent).await;

    // ready -> implementing requires the claim; without a fence it teaches.
    let (s, body) = app.transition(&app.worker, &parent, "implementing").await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "fence.required");

    let (s, body) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{parent}/transition"),
            json!({ "to": "implementing", "fence": fence }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let (s, body) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{parent}/transition"),
            json!({ "to": "review", "fence": fence }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let (s, _) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{parent}/release"),
            json!({ "fence": fence, "reason": "PR open" }),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // review -> done blocked by the open child, naming it.
    let (s, body) = app.transition(&app.human, &parent, "done").await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "transition.guard");
    assert!(body["message"].as_str().unwrap().contains(&child));
    assert_eq!(body["details"]["offending_tickets"][0], child.as_str());

    // Close the child, then done passes.
    let (s, b) = app.transition(&app.admin, &child, "cancelled").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, body) = app.transition(&app.human, &parent, "done").await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "done");
    assert_eq!(body["state_category"], "done");
}

/// `guard:has_link:commit` makes "done" prove itself: without a commit link the
/// transition 409s naming the missing key (not ticket ids), and passes once the
/// link is set. An empty value must not satisfy it — a blank string would make
/// the proof a formality.
#[tokio::test]
async fn has_link_guard_requires_proof_before_done() {
    let app = TestApp::spawn().await;
    let (s, body) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({
                "id": "proof",
                "name": "Proof required",
                "workflow": {
                    "name": "proof-wf",
                    "initial": "ready",
                    "states": [
                        { "id": "ready", "category": "todo", "claimable": true },
                        { "id": "review", "category": "review" },
                        { "id": "done", "category": "done", "terminal": true },
                        { "id": "cancelled", "category": "cancelled", "terminal": true }
                    ],
                    "transitions": [
                        { "from": "ready", "to": "review" },
                        { "from": "ready", "to": "cancelled" },
                        { "from": "review", "to": "done", "requires": ["guard:has_link:commit"] },
                        { "from": "review", "to": "cancelled" }
                    ]
                }
            }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");

    let (s, t) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "proof", "title": "Work that must prove itself" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let id = t["id"].as_str().unwrap().to_string();
    let (s, b) = app.transition(&app.admin, &id, "review").await;
    assert_eq!(s, StatusCode::OK, "{b}");

    // No commit link: blocked, and the error teaches which key is missing.
    let (s, body) = app.transition(&app.admin, &id, "done").await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "transition.guard");
    assert_eq!(body["details"]["guard"], "has_link:commit");
    // The subject is this ticket, so no offender list is invented.
    assert!(
        body["details"]["offending_tickets"].is_null(),
        "has_link failures must not name ticket ids: {body}"
    );
    let msg = body["message"].as_str().unwrap();
    assert!(msg.contains("commit"), "{msg}");
    let remedy = body["remedy"].as_str().unwrap();
    assert!(remedy.contains("takomo_link"), "{remedy}");

    // A blank value is not proof.
    let (s, _) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "links": { "commit": "   " } }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, body) = app.transition(&app.admin, &id, "done").await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "transition.guard");

    // A real sha satisfies it.
    let (s, _) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "links": { "commit": "5caea2a0f3b91c7d4e28a6b5f0c1d9e8a7b6c5d4" } }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, body) = app.transition(&app.admin, &id, "done").await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "done");
}

#[tokio::test]
async fn has_link_guard_rejects_whitespace_around_the_key_at_upload() {
    let app = TestApp::spawn().await;
    let (s, body) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({
                "id": "spc",
                "name": "Whitespace guard",
                "workflow": {
                    "name": "spc-wf",
                    "initial": "ready",
                    "states": [
                        { "id": "ready", "category": "todo", "claimable": true },
                        { "id": "done", "category": "done", "terminal": true }
                    ],
                    "transitions": [
                        { "from": "ready", "to": "done", "requires": ["guard:has_link: commit"] }
                    ]
                }
            }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "workflow.invalid");
    let problems = body["details"]["problems"].as_array().unwrap();
    assert!(
        problems
            .iter()
            .any(|p| p.as_str().unwrap().contains("whitespace")),
        "must teach about the whitespace trap: {body}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn event_cursor_orders_and_longpoll_wakes() {
    let app = TestApp::spawn().await;
    app.create_ticket("Event log seed one").await;
    app.create_ticket("Event log seed two").await;

    // Cursor read: strictly increasing seqs, cursor = last seq.
    let (s, page) = app.get(&app.admin, "/v1/events?since=0").await;
    assert_eq!(s, StatusCode::OK);
    let events = page["events"].as_array().unwrap();
    assert!(events.len() >= 3, "workflow_changed + 2x created expected");
    let seqs: Vec<i64> = events.iter().map(|e| e["seq"].as_i64().unwrap()).collect();
    for pair in seqs.windows(2) {
        assert!(pair[0] < pair[1], "seqs must strictly increase: {seqs:?}");
    }
    let cursor = page["cursor"].as_i64().unwrap();
    assert_eq!(cursor, *seqs.last().unwrap());

    // Resuming from the cursor yields nothing (wait=0).
    let (_, page2) = app
        .get(&app.admin, &format!("/v1/events?since={cursor}"))
        .await;
    assert!(page2["events"].as_array().unwrap().is_empty());
    assert_eq!(page2["cursor"].as_i64().unwrap(), cursor);

    // Long-poll: a waiting reader is woken by the next write.
    let waiter = {
        let client = app.client.clone();
        let base = app.base.clone();
        let token = app.admin.clone();
        tokio::spawn(async move {
            let start = Instant::now();
            let resp = client
                .get(format!("{base}/v1/events?since={cursor}&wait=15"))
                .bearer_auth(token)
                .send()
                .await
                .unwrap();
            (start.elapsed(), resp.json::<Value>().await.unwrap())
        })
    };
    tokio::time::sleep(Duration::from_millis(400)).await;
    let id = app.create_ticket("Long poll wake trigger").await;

    let (elapsed, page3) = waiter.await.unwrap();
    assert!(
        elapsed < Duration::from_secs(10),
        "long-poll should wake promptly, took {elapsed:?}"
    );
    let events = page3["events"].as_array().unwrap();
    assert!(!events.is_empty());
    assert!(events
        .iter()
        .any(|e| e["kind"] == "created" && e["ticket"] == id.as_str()));
    assert!(page3["cursor"].as_i64().unwrap() > cursor);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ready_claim_longpoll_wakes_on_new_work() {
    let app = TestApp::spawn().await;
    let waiter = {
        let client = app.client.clone();
        let base = app.base.clone();
        let token = app.worker.clone();
        tokio::spawn(async move {
            let start = Instant::now();
            let resp = client
                .post(format!("{base}/v1/ready/claim"))
                .bearer_auth(token)
                .json(&json!({ "project": "tp", "wait_seconds": 15 }))
                .send()
                .await
                .unwrap();
            let status = resp.status();
            (
                start.elapsed(),
                status,
                resp.json::<Value>().await.unwrap_or(Value::Null),
            )
        })
    };
    tokio::time::sleep(Duration::from_millis(400)).await;
    let id = app
        .create_ticket("Work arriving while a worker waits")
        .await;
    // brief -> spec: spec is claimable in factory-default, so this wakes the
    // waiting claimer.
    let (s, b) = app.transition(&app.human, &id, "spec").await;
    assert_eq!(s, StatusCode::OK, "{b}");

    let (elapsed, status, body) = waiter.await.unwrap();
    assert_eq!(
        status,
        StatusCode::OK,
        "waiter should get the ticket: {body}"
    );
    assert_eq!(body["id"], id.as_str());
    assert_eq!(body["state"], "spec");
    assert!(elapsed < Duration::from_secs(10), "took {elapsed:?}");
}

#[tokio::test]
async fn write_rate_limit_returns_429_with_retry_after() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Rate limit target").await;

    // The store the server owns is not reachable from here, so mint through a
    // second connection to the same DB file — the CLI's root of trust.
    let tight = app.mint_limited("agent:chatty", &["read", "write"], None, 3);

    let mut last = None;
    for i in 0..4 {
        let resp = app
            .authed(Method::POST, &tight, &format!("/v1/tickets/{id}/comments"))
            .json(&json!({ "body": format!("comment {i}") }))
            .send()
            .await
            .unwrap();
        last = Some(resp);
    }
    let resp = last.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry_after: i64 = resp
        .headers()
        .get("Retry-After")
        .expect("Retry-After header")
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1..=60).contains(&retry_after));
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["code"], "rate.limited");
    let message = body["message"].as_str().expect("message");
    assert!(
        message.contains("write budget of 3 writes/minute"),
        "429 names the budget the caller actually spent: {message}"
    );
    assert!(
        message.contains("reads are free"),
        "429 tells the caller reads still work: {message}"
    );
    assert!(
        body["remedy"].as_str().is_some_and(|r| r.contains("Wait")),
        "429 carries a remedy: {body}"
    );

    // …and that is true: a GET is not charged, so reads keep working while the
    // write budget is exhausted.
    let (status, _) = app.get(&tight, &format!("/v1/tickets/{id}")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "reads stay free while writes are limited"
    );
}

// ---------------------------------------------------------------------------
// Ticket field lists (takomo-ammj).
//
// One ticket field has to be restated in six independent places:
//
//   1. `CREATE TABLE tickets` in SCHEMA + a `migrate()` ALTER  src/store/mod.rs
//   2. `TICKET_COLS` + `row_to_ticket`                     src/store/helpers.rs
//   3. `struct Ticket` + `Ticket::to_json`                   src/store/model.rs
//   4. the INSERT column list + `TicketCreate` / `TicketPatch` + a
//      `patch_ticket` branch                               src/store/tickets.rs
//   5. `CREATE_FIELDS` + `PATCH_FIELDS`                       src/api/tickets.rs
//   6. `NewArgs`, and the `Ticket` / `TicketCreate` / `TicketPatch` schemas
//                                            src/mcp.rs + spec/openapi.yaml
//
// Not one of the misses is a compile error, which is the actual defect: omit the
// field from (2) and `row_to_ticket` fails at runtime; from (3) and it silently
// never reaches the wire; from (5) and the API 400s a field the store accepts.
//
// The tests below are the wall. They read each list at runtime and compare them,
// so they pin the *invariant* — every list agrees — rather than today's field
// set: a field added to all six keeps them green, a field added to five fails
// naming the sixth. Where a mismatch is deliberate it is an entry in one of the
// three tables here, with the reason, instead of a silent gap.
// ---------------------------------------------------------------------------

/// Columns of `tickets` that deliberately never appear in the ticket JSON.
const COLUMNS_OFF_THE_WIRE: [(&str, &str); 5] = [
    (
        "claim_holder",
        "folded into the `claim` object by Ticket::to_json",
    ),
    (
        "claim_expires_at",
        "folded into the `claim` object by Ticket::to_json",
    ),
    (
        "claim_since",
        "folded into the `claim` object by Ticket::to_json (as `claim.since`)",
    ),
    (
        "lapsed_claim_holder",
        "an internal continuity marker for lease re-entry; next to `claim: null` a stale holder would read as a live claim, and the same fact is already on the wire as the lease_expired event's holder",
    ),
    (
        "fence_seq",
        "handed out by claim/heartbeat responses only, so a reader cannot mistake it for a lease it holds",
    ),
];

/// Ticket JSON keys that are not columns of `tickets`, and where each comes from.
const KEYS_WITHOUT_A_COLUMN: [(&str, &str); 3] = [
    (
        "state_category",
        "joined from workflow_states by TICKET_COLS",
    ),
    ("blocked_by", "read from the deps table by load_blocked_by"),
    (
        "claim",
        "derived from claim_holder + claim_expires_at + claim_since",
    ),
];

/// Ticket JSON keys no client may set, and why. Everything else on the wire has
/// to be reachable through `CREATE_FIELDS` or `PATCH_FIELDS` — otherwise it is a
/// field the API shows but nobody can fill.
const KEYS_THE_SERVER_OWNS: [(&str, &str); 11] = [
    ("id", "generated by generate_ticket_id"),
    ("state_category", "derived from the project workflow"),
    ("claim", "set by claim/heartbeat/release"),
    ("version", "bumped by touch_ticket; the ETag value"),
    ("created_by", "the token's actor"),
    ("created_at", "server clock"),
    ("updated_at", "server clock"),
    ("archived_at", "set by POST /tickets/{id}/archive|unarchive"),
    (
        "schedule",
        "set only when a schedule materializes the ticket; a client naming its own \
         would be claiming a provenance that never happened",
    ),
    (
        "occurrence",
        "the slot the schedule fired, and half of the UNIQUE(schedule, occurrence) \
         index — a client-set value could forge or block an occurrence",
    ),
    (
        "expires_at",
        "computed from the schedule's cadence at materialization. Client-settable \
         it would be a way to un-expire work without touching the cadence, which \
         is the maintenance agent's job to decide, not the caller's",
    ),
];

fn sorted_strings<'a>(items: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = items.into_iter().map(str::to_string).collect();
    out.sort();
    out
}

/// The sorted keys of a JSON object. Empty is a hard error: a guard comparing
/// two empty lists passes while checking nothing.
fn object_keys(value: &Value, what: &str) -> Vec<String> {
    let obj = value
        .as_object()
        .unwrap_or_else(|| panic!("{what} is not a JSON object: {value}"));
    assert!(
        !obj.is_empty(),
        "{what} has no keys — this guard would be comparing nothing"
    );
    sorted_strings(obj.keys().map(String::as_str))
}

/// Property names of `components.schemas.<name>` in the spec. `include_str!`
/// means a moved or renamed spec fails the build instead of quietly checking
/// nothing.
fn openapi_properties(schema: &str) -> Vec<String> {
    let spec: Value = serde_norway::from_str(include_str!("../spec/openapi.yaml"))
        .expect("spec/openapi.yaml parses as YAML");
    object_keys(
        &spec["components"]["schemas"][schema]["properties"],
        &format!("spec/openapi.yaml components.schemas.{schema}.properties"),
    )
}

/// The `tickets` columns the running server's database actually has, i.e. list 1
/// (SCHEMA plus whatever `migrate()` added) as SQLite sees it.
fn tickets_table_columns(app: &TestApp) -> Vec<String> {
    let conn = rusqlite::Connection::open(app.db_path()).expect("open db");
    let mut stmt = conn
        .prepare("PRAGMA table_info(tickets)")
        .expect("prepare table_info");
    let cols: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(1))
        .expect("query table_info")
        .collect::<Result<_, _>>()
        .expect("read column names");
    assert!(
        cols.len() > 5,
        "PRAGMA table_info(tickets) returned {} columns — the table this guard reads is gone",
        cols.len()
    );
    sorted_strings(cols.iter().map(String::as_str))
}

/// The ticket JSON keys a client sees, taken from the **list** endpoint: list
/// items are exactly `Ticket::to_json`, while create adds `similar` and the
/// single-ticket read can attach convention hints.
async fn ticket_wire_keys(app: &TestApp) -> Vec<String> {
    let id = app.create_ticket("Wire shape probe").await;
    let (s, list) = app.get(&app.admin, "/v1/tickets?project=tp").await;
    assert_eq!(s, StatusCode::OK, "{list}");
    let item = list["items"]
        .as_array()
        .expect("items array")
        .iter()
        .find(|t| t["id"] == id.as_str())
        .unwrap_or_else(|| panic!("{id} missing from the ticket list: {list}"))
        .clone();
    object_keys(&item, "the ticket JSON")
}

/// Lists 1, 2 and 3 at once: every column of `tickets` reaches the wire, and
/// every wire key comes from a column. Both ends are read off the running
/// server, so the whole path — SCHEMA, `TICKET_COLS`, `row_to_ticket`, `struct
/// Ticket`, `Ticket::to_json` — is exercised rather than pattern-matched.
#[tokio::test]
async fn every_ticket_column_reaches_the_wire() {
    let app = TestApp::spawn().await;
    let columns = tickets_table_columns(&app);
    let wire = ticket_wire_keys(&app).await;

    // A stale exemption is a hole: check each entry still describes reality.
    for (col, why) in COLUMNS_OFF_THE_WIRE {
        let col = col.to_string();
        assert!(
            columns.contains(&col),
            "COLUMNS_OFF_THE_WIRE exempts `{col}` ({why}) but `tickets` has no such column — \
             drop the entry rather than leaving an exemption nothing checks"
        );
        assert!(
            !wire.contains(&col),
            "COLUMNS_OFF_THE_WIRE calls `{col}` internal ({why}), but it is on the wire now — \
             drop the entry"
        );
    }
    for (key, from) in KEYS_WITHOUT_A_COLUMN {
        assert!(
            wire.contains(&key.to_string()),
            "KEYS_WITHOUT_A_COLUMN lists `{key}` ({from}) but no such key is on the wire — \
             drop the entry"
        );
    }

    for col in &columns {
        if COLUMNS_OFF_THE_WIRE.iter().any(|(c, _)| c == col) {
            continue;
        }
        assert!(
            wire.contains(col),
            "the `tickets` table has a column `{col}` that never reaches the ticket JSON. A \
             field only arrives if it is in ALL of: TICKET_COLS and row_to_ticket \
             (src/store/helpers.rs), struct Ticket and Ticket::to_json (src/store/model.rs) — \
             one of those is missing `{col}`. If the column is deliberately internal, add it to \
             COLUMNS_OFF_THE_WIRE with the reason."
        );
    }
    for key in &wire {
        if KEYS_WITHOUT_A_COLUMN.iter().any(|(k, _)| k == key) {
            continue;
        }
        assert!(
            columns.contains(key),
            "the ticket JSON carries `{key}` but `tickets` has no such column. Add the column to \
             SCHEMA plus a migrate() branch (src/store/mod.rs) so older databases get it too, or \
             — if the value is derived — record where it comes from in KEYS_WITHOUT_A_COLUMN."
        );
    }
}

/// Half of list 6: the wire shape and the spec's `Ticket` schema name the same
/// properties. The spec is the contract and it drifts silently, so this is the
/// only thing that notices.
#[tokio::test]
async fn ticket_wire_shape_matches_the_openapi_ticket_schema() {
    let app = TestApp::spawn().await;
    let wire = ticket_wire_keys(&app).await;
    assert_eq!(
        wire,
        openapi_properties("Ticket"),
        "left = the properties GET /v1/tickets returns, right = spec/openapi.yaml \
         components.schemas.Ticket. They must match: add the field to whichever side lacks it."
    );
}

/// Lists 4, 5 and the request half of 6: the store structs, the API's accepted
/// field arrays and the spec's request schemas all name the same fields. The
/// structs are enumerable because `TicketCreate`/`TicketPatch` derive
/// `Serialize` for this purpose (see their doc comments) — nothing serializes
/// them at runtime.
#[test]
fn ticket_request_field_lists_match_their_structs_and_the_spec() {
    use takomo::api::tickets::{CREATE_FIELDS, PATCH_FIELDS};
    use takomo::store::{TicketCreate, TicketPatch};

    for (shape, from_struct, from_api) in [
        (
            "TicketCreate",
            object_keys(
                &serde_json::to_value(TicketCreate::default()).expect("serialize TicketCreate"),
                "store::TicketCreate",
            ),
            sorted_strings(CREATE_FIELDS),
        ),
        (
            "TicketPatch",
            object_keys(
                &serde_json::to_value(TicketPatch::default()).expect("serialize TicketPatch"),
                "store::TicketPatch",
            ),
            sorted_strings(PATCH_FIELDS),
        ),
    ] {
        assert_eq!(
            from_struct, from_api,
            "left = the fields store::{shape} carries, right = the fields src/api/tickets.rs \
             accepts. A field only in the struct is unreachable over HTTP (the handler's \
             reject_unknown_fields 400s it); a field only in the array is parsed and dropped."
        );
        assert_eq!(
            from_api,
            openapi_properties(shape),
            "left = the fields src/api/tickets.rs accepts, right = spec/openapi.yaml \
             components.schemas.{shape}. The spec is the contract — update it in the same change."
        );
    }

    // The MCP door is a subset check, not equality: what it catches is an
    // argument MCP accepts that the REST create body cannot express. `state` is
    // intentionally REST-only; `idempotency_key` maps to the `Idempotency-Key`
    // header, not a JSON body field.
    let new_args = serde_json::to_value(rmcp::schemars::schema_for!(takomo::mcp::NewArgs))
        .expect("serialize NewArgs schema");
    let mcp_only = ["idempotency_key"];
    for arg in object_keys(&new_args["properties"], "takomo_new's MCP input schema") {
        if mcp_only.contains(&arg.as_str()) {
            continue;
        }
        assert!(
            sorted_strings(CREATE_FIELDS).contains(&arg),
            "src/mcp.rs NewArgs takes `{arg}`, which CREATE_FIELDS in src/api/tickets.rs does \
             not accept — the MCP tool would pass a field the REST create rejects"
        );
    }
}

/// The other half of list 4: a field can be in every array and still be dropped
/// by the INSERT column list or missing a `patch_ticket` branch. So set every
/// creatable and every patchable field for real and read the result back.
///
/// The two request bodies double as the coverage table — each is asserted to
/// name exactly `CREATE_FIELDS` / `PATCH_FIELDS`, so a new field cannot be added
/// to those arrays without being exercised here.
#[tokio::test]
async fn every_creatable_and_patchable_field_round_trips() {
    let app = TestApp::spawn().await;
    let parent = app.create_typed("Round-trip parent", "epic", None).await;
    let blocker = app.create_ticket("Round-trip blocker").await;
    let new_parent = app.create_typed("Round-trip re-parent", "epic", None).await;

    let create = json!({
        "project": "tp",
        "type": "bug",
        "parent": parent,
        "title": "Round-trip subject",
        "body": "created body",
        "priority": "high",
        "labels": ["from-create"],
        "tags": ["component:billing"],
        "metadata": { "test.created": "yes" },
        "blocked_by": [blocker],
        "state": "brief",
    });
    assert_eq!(
        object_keys(&create, "the create request below"),
        sorted_strings(takomo::api::tickets::CREATE_FIELDS),
        "this request must set every field POST /v1/tickets accepts, or the field missing from \
         the left is never proven to survive the INSERT — add it here with an assertion below"
    );

    let (s, t) = app.post(&app.admin, "/v1/tickets", create).await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let id = t["id"].as_str().expect("ticket id").to_string();
    assert_eq!(t["project"], "tp");
    assert_eq!(t["type"], "bug");
    assert_eq!(t["parent"], parent.as_str());
    assert_eq!(t["title"], "Round-trip subject");
    assert_eq!(t["body"], "created body");
    assert_eq!(t["priority"], "high");
    assert_eq!(t["labels"], json!(["from-create"]));
    assert_eq!(t["tags"], json!(["component:billing"]));
    assert_eq!(t["metadata"]["test.created"], "yes");
    assert_eq!(t["blocked_by"], json!([blocker]));
    assert_eq!(t["state"], "brief", "the requested initial state");

    // Re-read: create answers from a fresh SELECT, but only a second read proves
    // the row itself carries every field rather than the request echoing back.
    let (s, stored) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
    assert_eq!(s, StatusCode::OK, "{stored}");
    for key in [
        "type", "parent", "title", "body", "priority", "labels", "tags",
    ] {
        assert_eq!(
            stored[key], t[key],
            "`{key}` did not survive create -> storage -> read; check the INSERT column list in \
             src/store/tickets.rs"
        );
    }

    let version = stored["version"].as_i64().expect("version");
    let patch = json!({
        "title": "Round-trip patched",
        "body": "patched body",
        "priority": "low",
        "labels": ["keep", "drop"],
        "labels_add": ["added"],
        "labels_remove": ["drop"],
        "tags": ["component:api", "component:web"],
        "tags_add": ["person:ada"],
        "tags_remove": ["component:web"],
        "parent": new_parent,
        "links": { "pr": "https://example.test/pr/1" },
        "metadata_merge": { "test.patched": "yes" },
        "fence": 0,
    });
    assert_eq!(
        object_keys(&patch, "the patch request below"),
        sorted_strings(takomo::api::tickets::PATCH_FIELDS),
        "this request must set every field PATCH /v1/tickets/{{id}} accepts, or the field missing \
         from the left has no proof its patch_ticket branch exists — add it here with an \
         assertion below"
    );

    // `body` is the one field under CAS, so the whole patch carries If-Match.
    let (s, p) = app
        .patch_with(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            &[("If-Match", &format!("\"{version}\""))],
            patch,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!(p["title"], "Round-trip patched");
    assert_eq!(p["body"], "patched body");
    assert_eq!(p["priority"], "low");
    assert_eq!(
        p["labels"],
        json!(["keep", "added"]),
        "labels replaced, then labels_add applied, then labels_remove"
    );
    assert_eq!(
        p["tags"],
        json!(["component:api", "person:ada"]),
        "tags replaced, then tags_add applied, then tags_remove"
    );
    assert_eq!(p["parent"], new_parent.as_str());
    assert_eq!(p["links"]["pr"], "https://example.test/pr/1");
    assert_eq!(p["metadata"]["test.patched"], "yes");
    assert_eq!(
        p["metadata"]["test.created"], "yes",
        "metadata_merge merges rather than replaces"
    );

    let (s, again) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
    assert_eq!(s, StatusCode::OK, "{again}");
    for key in [
        "title", "body", "priority", "labels", "tags", "parent", "links",
    ] {
        assert_eq!(
            again[key], p[key],
            "`{key}` did not survive patch -> storage -> read; check its patch_ticket branch in \
             src/store/tickets.rs"
        );
    }
}

/// Every wire field must be reachable through create or patch, or it is a field
/// the API shows and nobody can fill. The server-owned keys are the documented
/// exception.
#[test]
fn every_client_settable_ticket_field_is_writable_somewhere() {
    // The wire key set is the spec's `Ticket` schema; the test above pins the two
    // to each other, so reading it here needs no server.
    let writable = {
        let mut w = sorted_strings(takomo::api::tickets::CREATE_FIELDS);
        w.extend(sorted_strings(takomo::api::tickets::PATCH_FIELDS));
        w
    };
    for key in openapi_properties("Ticket") {
        if let Some((_, why)) = KEYS_THE_SERVER_OWNS.iter().find(|(k, _)| *k == key) {
            assert!(
                !writable.contains(&key),
                "KEYS_THE_SERVER_OWNS says `{key}` is server-owned ({why}), but create or patch \
                 accepts it — drop the entry"
            );
            continue;
        }
        assert!(
            writable.contains(&key),
            "ticket field `{key}` is readable but no client can set it: it is in neither \
             CREATE_FIELDS nor PATCH_FIELDS (src/api/tickets.rs). Add it to whichever fits, or \
             record in KEYS_THE_SERVER_OWNS why the server owns it."
        );
    }
}

/// Sparse projection needs no per-field list — `project_fields` filters whatever
/// keys the ticket JSON has — so what has to be pinned is that it stays that
/// way, for every field, with `id` always present.
#[tokio::test]
async fn sparse_field_projection_covers_every_ticket_field() {
    let app = TestApp::spawn().await;
    let wire = ticket_wire_keys(&app).await;
    for key in &wire {
        let (s, list) = app
            .get(&app.admin, &format!("/v1/tickets?project=tp&fields={key}"))
            .await;
        assert_eq!(s, StatusCode::OK, "{list}");
        let item = list["items"]
            .as_array()
            .and_then(|items| items.first())
            .unwrap_or_else(|| panic!("?fields={key} returned no items: {list}"))
            .clone();
        let mut want = vec!["id".to_string()];
        if key != "id" {
            want.push(key.clone());
            want.sort();
        }
        assert_eq!(
            object_keys(&item, "the projected ticket"),
            want,
            "`?fields={key}` must return exactly that field plus `id`"
        );
    }
}

// ---------------------------------------------------------------------------
// The error-code vocabulary (takomo-b6cg / takomo-a0wi).
//
// CLAUDE.md: "Errors are part of the contract — a stable machine `code`". They
// were not. 133 distinct codes were minted at 274 sites as bare `&str` literals
// and 104 of them appeared nowhere in spec/openapi.yaml, so no client could be
// written against the documented set and a rename was grep-and-pray.
//
// The proof it was decaying rather than merely stale: `heartbeat.no_lease`
// landed in src/mcp.rs in d350c26 and reached spec/openapi.yaml zero times,
// green through both `route-test-pairing` and `openapi-current` — because those
// check *file pairing* (did a tests/ file and the spec change alongside the
// surface), never whether the codes themselves are covered.
//
// So: `x-error-codes` at the root of spec/openapi.yaml is the vocabulary, and
// the two tests below are the wall.
//
// `every_emitted_error_code_is_documented_in_the_spec` compares the codes the
// scan finds under src/ against that list, in both directions — a code emitted
// and not listed is red, and so is a listed code nothing emits any more, which
// is what keeps the list from filling with fiction after a rename.
//
// `the_error_code_scan_can_see_every_construction_site` guards the scan itself,
// which is the part that matters. A gate that silently misses codes is worse
// than no gate, because it licenses the belief that coverage is complete. It
// fails on a new `ApiError` constructor the scan does not know how to read, on a
// call site whose code argument is not a literal and is not accounted for below,
// and on an error body built anywhere but src/error.rs.
//
// What the scan reads and what it cannot ---------------------------------
//
// It is a static scan of the `ApiError::*` call sites, not a registry of
// constants that constructors must go through. A registry is sturdier — the
// compiler would enforce it — but it means rewriting 274 literals across
// src/api/, src/store/ and src/mcp.rs, and the value here is the gate, not the
// refactor. The trade-off is worth stating plainly, because it decides what a
// green run does and does not mean. Filed as takomo-ooxm.
//
// Green means: every code assembled from a string literal at an `ApiError`
// constructor is documented. That is all 133 of today's codes.
//
// Green does NOT mean:
//
//   - a code assembled at runtime is covered. `ApiError::not_found` builds
//     `notfound.{kind}` by `format!`, so the family is enumerated from the
//     *kind* literal at each call site instead — pass a non-literal kind and the
//     scan cannot see the code. That is not silent: the site lands in
//     `CODES_THE_SCAN_CANNOT_READ` or the build fails.
//   - errors from outside this crate are covered. axum's own 404/405 for an
//     unrouted path and rmcp's JSON-RPC frame errors carry no takomo `code` and
//     are not in the vocabulary.
//   - the *status* a code rides on is checked. That lives per route in the
//     spec's `responses`, and only the code names are compared here.
//
// It walks src/ on disk rather than `include_str!`-ing a fixed file list, so a
// new module is scanned the day it is added instead of being a silent hole.
// ---------------------------------------------------------------------------

/// Where the `code` sits in an `ApiError` constructor's argument list.
#[derive(Clone, Copy)]
enum CodeArg {
    /// Second argument: `ApiError::new(status, code, message)`.
    AfterStatus,
    /// First argument: `ApiError::validation(code, message)`.
    First,
    /// First argument is the *kind*, which the constructor formats into
    /// `notfound.<kind>`.
    FirstIsNotFoundKind,
    /// No code argument — the constructor hard-codes this one.
    Fixed(&'static str),
    /// Delegates to another constructor, so it mints no code of its own.
    Delegates,
}

/// Every associated function on `ApiError` that a caller can reach. The scan
/// fails on any name not in here, so adding a constructor cannot quietly hide a
/// family of codes: the build asks you to teach the scan where its code is.
const ERROR_CTORS: [(&str, CodeArg); 7] = [
    ("new", CodeArg::AfterStatus),
    ("validation", CodeArg::First),
    ("conflict", CodeArg::First),
    ("bad_request", CodeArg::First),
    ("not_found", CodeArg::FirstIsNotFoundKind),
    ("internal", CodeArg::Fixed("internal")),
    // The From<rusqlite::Error> impl, which calls ApiError::internal — already
    // counted at that call site.
    ("from", CodeArg::Delegates),
];

/// Call sites whose code argument is not a string literal, so the scan cannot
/// read a code off them, with why that is fine. Keyed by file and by the
/// argument's source text rather than a line number, so moving code around does
/// not churn the table. A stale entry fails: the reason has to still be true.
const CODES_THE_SCAN_CANNOT_READ: [(&str, &str, &str); 2] = [
    (
        "src/error.rs",
        "&format!(\"notfound.{kind}\")",
        "ApiError::not_found's own body. The notfound.* family is enumerated from \
         the `kind` literal at each of its call sites instead, which is why \
         not_found is itself scanned as a code constructor.",
    ),
    (
        "src/error.rs",
        "code",
        "ApiError::validation / conflict / bad_request forward their caller's code \
         to ApiError::new unchanged, so their call sites are where the scan reads it.",
    ),
];

/// Codes deliberately kept out of `x-error-codes`, with the reason. Empty, and
/// that is the finding rather than an oversight: every code the scan finds today
/// reaches a client on the wire, so every one of them is contract. The table is
/// here so the first deliberate exemption has an obvious home and a reason
/// attached — and a stale entry fails too, so it cannot outlive its code.
const CODES_OFF_THE_CONTRACT: [(&str, &str); 0] = [];

/// One `ApiError` construction the scan resolved.
struct CodeSite {
    code: String,
    /// `src/…:<line>`, for the failure message.
    at: String,
}

/// A construction whose code argument the scan could not resolve to a literal.
struct OpaqueSite {
    file: String,
    /// The argument's source text, whitespace-collapsed.
    arg: String,
    at: String,
}

struct ErrorCodeScan {
    sites: Vec<CodeSite>,
    opaque: Vec<OpaqueSite>,
    /// Constructor names found that `ERROR_CTORS` does not describe.
    unknown_ctors: Vec<String>,
    files: usize,
}

impl ErrorCodeScan {
    /// Codes to their first construction site, for a failure message that points
    /// somewhere.
    fn by_code(&self) -> std::collections::BTreeMap<&str, &str> {
        let mut out = std::collections::BTreeMap::new();
        for site in &self.sites {
            out.entry(site.code.as_str()).or_insert(site.at.as_str());
        }
        out
    }
}

/// Every `.rs` file under `src/`, walked rather than listed so a new module is
/// scanned the day it lands.
fn rust_sources() -> Vec<(String, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![root.clone()];
    let mut out = Vec::new();
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
            .flatten()
        {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let rel = path
                    .strip_prefix(root.parent().expect("src has a parent"))
                    .expect("under the manifest dir")
                    .to_string_lossy()
                    .replace('\\', "/");
                let text = std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
                out.push((rel, text));
            }
        }
    }
    out.sort();
    out
}

/// The source text of the argument list's first argument, and the rest after its
/// comma. Tracks bracket depth and skips over string literals, so a comma inside
/// a `format!` argument or a message does not end the argument early.
fn split_first_arg(args: &str) -> Option<(&str, &str)> {
    let bytes = args.as_bytes();
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                if depth == 0 {
                    // End of the argument list: a single-argument call.
                    return Some((&args[..i], ""));
                }
                depth -= 1;
            }
            b',' if depth == 0 => return Some((&args[..i], &args[i + 1..])),
            _ => {}
        }
        i += 1;
    }
    None
}

/// The contents of a leading `"…"` literal, when the expression is exactly that
/// and nothing else — no `&format!`, no concatenation, no constant. A code is
/// only counted when it can be read off the source with certainty.
fn plain_string_literal(expr: &str) -> Option<&str> {
    let expr = expr.trim();
    let inner = expr.strip_prefix('"')?.strip_suffix('"')?;
    // A second unescaped quote would mean this is not one literal.
    if inner.contains('"') || inner.contains('\\') {
        return None;
    }
    // Interpolation or anything outside the code alphabet: not a fixed code.
    inner
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
        .then_some(inner)
}

/// Scan `src/` for `ApiError` constructions and the codes they mint.
fn scan_error_codes() -> ErrorCodeScan {
    let mut scan = ErrorCodeScan {
        sites: Vec::new(),
        opaque: Vec::new(),
        unknown_ctors: Vec::new(),
        files: 0,
    };
    for (file, text) in rust_sources() {
        scan.files += 1;
        let mut from = 0usize;
        while let Some(hit) = text[from..].find("ApiError::") {
            let start = from + hit;
            let after = start + "ApiError::".len();
            let name_len = text[after..]
                .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .unwrap_or(0);
            let name = &text[after..after + name_len];
            from = after + name_len;
            // Only a call constructs anything; `-> ApiError` and the like carry
            // no parenthesis and mint nothing.
            if !text[from..].starts_with('(') {
                continue;
            }
            let line = text[..start].matches('\n').count() + 1;
            let at = format!("{file}:{line}");
            let Some((_, arg)) = ERROR_CTORS.iter().find(|(n, _)| *n == name) else {
                scan.unknown_ctors.push(format!("ApiError::{name} at {at}"));
                continue;
            };
            let args = &text[from + 1..];
            let code_expr = match arg {
                CodeArg::Fixed(code) => {
                    scan.sites.push(CodeSite {
                        code: (*code).to_string(),
                        at,
                    });
                    continue;
                }
                CodeArg::Delegates => continue,
                CodeArg::First | CodeArg::FirstIsNotFoundKind => {
                    split_first_arg(args).map(|(first, _)| first)
                }
                CodeArg::AfterStatus => split_first_arg(args)
                    .and_then(|(_, rest)| split_first_arg(rest).map(|(second, _)| second)),
            };
            let Some(code_expr) = code_expr else {
                panic!(
                    "the error-code scan could not find the code argument of \
                     ApiError::{name} at {at} — its argument list does not parse. \
                     Fix split_first_arg in tests/api.rs rather than leaving the \
                     site unread."
                );
            };
            match plain_string_literal(code_expr) {
                Some(literal) => scan.sites.push(CodeSite {
                    code: match arg {
                        CodeArg::FirstIsNotFoundKind => format!("notfound.{literal}"),
                        _ => literal.to_string(),
                    },
                    at,
                }),
                None => scan.opaque.push(OpaqueSite {
                    file: file.clone(),
                    arg: code_expr.split_whitespace().collect::<Vec<_>>().join(" "),
                    at,
                }),
            }
        }
    }
    scan
}

/// The keys of `x-error-codes` in the spec, i.e. the documented vocabulary.
fn spec_error_codes() -> Vec<String> {
    let spec: Value = serde_norway::from_str(include_str!("../spec/openapi.yaml"))
        .expect("spec/openapi.yaml parses as YAML");
    let documented = object_keys(&spec["x-error-codes"], "spec/openapi.yaml x-error-codes");
    for code in &documented {
        let description = spec["x-error-codes"][code].as_str().unwrap_or_default();
        assert!(
            description.len() > 20,
            "`x-error-codes.{code}` in spec/openapi.yaml has no useful description \
             ({description:?}). A code listed with nothing said about it documents \
             nothing — say what is wrong and what the caller should do."
        );
    }
    documented
}

/// The set of codes src/ emits and the set spec/openapi.yaml documents have to
/// be the same set. Both directions matter: an undocumented code is a contract
/// nobody can code against, and a documented code nothing emits is fiction that
/// survived a rename.
#[test]
fn every_emitted_error_code_is_documented_in_the_spec() {
    let scan = scan_error_codes();
    let emitted = scan.by_code();
    let documented: std::collections::BTreeSet<String> = spec_error_codes().into_iter().collect();
    assert!(
        documented.len() > 100,
        "spec/openapi.yaml x-error-codes lists only {} codes — this guard is \
         comparing almost nothing. Has the section moved?",
        documented.len()
    );
    assert!(
        emitted.len() > 100 && scan.sites.len() > 200,
        "the scan found {} codes at {} sites across {} files under src/ — far too \
         few, so it is checking almost nothing. Something about how errors are \
         constructed changed and rust_sources/scan_error_codes did not keep up.",
        emitted.len(),
        scan.sites.len(),
        scan.files
    );

    let exempt: std::collections::BTreeMap<&str, &str> =
        CODES_OFF_THE_CONTRACT.iter().copied().collect();
    for (code, reason) in &exempt {
        assert!(
            emitted.contains_key(code),
            "CODES_OFF_THE_CONTRACT in tests/api.rs exempts `{code}` from the spec \
             ({reason}), but nothing under src/ emits it any more. Drop the entry — \
             a stale exemption is a hole held open for no reason."
        );
        assert!(
            !documented.contains(*code),
            "`{code}` is both exempt in CODES_OFF_THE_CONTRACT and documented in \
             x-error-codes. Pick one: if it belongs in the contract, drop the \
             exemption."
        );
    }

    let missing: Vec<String> = emitted
        .iter()
        .filter(|(code, _)| !documented.contains(**code) && !exempt.contains_key(**code))
        .map(|(code, at)| format!("{code} (first emitted at {at})"))
        .collect();
    assert!(
        missing.is_empty(),
        "these error codes are emitted under src/ but absent from `x-error-codes` \
         in spec/openapi.yaml:\n  {}\n\nErrors are part of the contract: add each \
         one to the vocabulary with what it means and what the caller should do. \
         If a code deliberately stays undocumented, record it in \
         CODES_OFF_THE_CONTRACT in tests/api.rs with the reason.",
        missing.join("\n  ")
    );

    let stale: Vec<&str> = documented
        .iter()
        .map(String::as_str)
        .filter(|code| !emitted.contains_key(code))
        .collect();
    assert!(
        stale.is_empty(),
        "`x-error-codes` in spec/openapi.yaml documents these codes, but nothing \
         under src/ emits them:\n  {}\n\nEither the emitter was removed or renamed \
         and the spec was not — clients are branching on a code they will never \
         see. Delete the entry, or restore the code.",
        stale.join("\n  ")
    );
}

/// The gate above is only worth its green if the scan can see every place a code
/// is minted. This is that check: it fails on a constructor the scan does not
/// know, on a code argument it cannot read that is not accounted for, and on an
/// error body assembled outside src/error.rs.
#[test]
fn the_error_code_scan_can_see_every_construction_site() {
    let scan = scan_error_codes();
    assert!(
        scan.files > 20,
        "the scan walked only {} files under src/ — the walk is broken",
        scan.files
    );
    assert!(
        scan.unknown_ctors.is_empty(),
        "the error-code scan met `ApiError` constructors it does not know how to \
         read:\n  {}\n\nEvery one mints a code, so an unknown constructor is a \
         family of codes the spec gate cannot see. Add it to ERROR_CTORS in \
         tests/api.rs saying where its code argument sits.",
        scan.unknown_ctors.join("\n  ")
    );

    // Sites whose code is not a literal, keyed the way the table is.
    let mut found: std::collections::BTreeMap<(&str, &str), Vec<&str>> =
        std::collections::BTreeMap::new();
    for site in &scan.opaque {
        found
            .entry((site.file.as_str(), site.arg.as_str()))
            .or_default()
            .push(site.at.as_str());
    }
    let accounted: std::collections::BTreeSet<(&str, &str)> = CODES_THE_SCAN_CANNOT_READ
        .iter()
        .map(|(file, arg, _)| (*file, *arg))
        .collect();

    let unaccounted: Vec<String> = found
        .iter()
        .filter(|(key, _)| !accounted.contains(*key))
        .map(|((file, arg), ats)| format!("{file}: code argument `{arg}` at {}", ats.join(", ")))
        .collect();
    assert!(
        unaccounted.is_empty(),
        "these `ApiError` constructions build their code from something the scan \
         cannot read, so the codes they can produce are invisible to \
         every_emitted_error_code_is_documented_in_the_spec:\n  {}\n\nPass a string \
         literal instead — or, if the code genuinely has to be assembled at \
         runtime, add the site to CODES_THE_SCAN_CANNOT_READ in tests/api.rs with \
         the reason and make sure every code it can produce is in x-error-codes.",
        unaccounted.join("\n  ")
    );

    let stale: Vec<String> = CODES_THE_SCAN_CANNOT_READ
        .iter()
        .filter(|(file, arg, _)| !found.contains_key(&(*file, *arg)))
        .map(|(file, arg, reason)| format!("{file}: `{arg}` — {reason}"))
        .collect();
    assert!(
        stale.is_empty(),
        "CODES_THE_SCAN_CANNOT_READ in tests/api.rs excuses sites that no longer \
         exist:\n  {}\n\nDrop the entries. An exemption that outlives its call site \
         is a hole the next dynamic code slips through unnoticed.",
        stale.join("\n  ")
    );

    // The scan reads `ApiError` call sites, which only covers the contract
    // because `ApiError` is the sole author of an error body. Nail that down.
    let builders: Vec<String> = rust_sources()
        .into_iter()
        .filter(|(_, text)| text.contains("ErrorBody {"))
        .map(|(file, _)| file)
        .collect();
    assert_eq!(
        builders,
        ["src/error.rs"],
        "an `ErrorBody` is built outside src/error.rs. The error-code scan reads \
         `ApiError` constructor call sites, so a body assembled directly carries a \
         code no gate can see. Route it through an ApiError constructor."
    );
}

// ---------------------------------------------------------------------------
// Error-code families (takomo-lycg) and event-kind pinning (takomo-oshg).
// ---------------------------------------------------------------------------

#[test]
fn lease_and_fence_family_is_documented_as_one_set() {
    let spec: Value = serde_norway::from_str(include_str!("../spec/openapi.yaml"))
        .expect("spec/openapi.yaml parses as YAML");
    let family = &spec["x-error-code-families"]["lease_and_fence"];
    assert!(
        family.is_object(),
        "spec/openapi.yaml must carry x-error-code-families.lease_and_fence"
    );
    let members = object_keys(&family["members"], "lease_and_fence.members");
    for code in takomo::error_codes::lease_and_fence::ALL {
        assert!(
            members.iter().any(|m| m == code),
            "lease_and_fence.members must list `{code}` — the family is how agents \
             discover the eight codes belong together"
        );
        assert!(
            spec["x-error-codes"][code].is_string(),
            "`x-error-codes.{code}` must describe the member"
        );
    }
    assert_eq!(
        members.len(),
        takomo::error_codes::lease_and_fence::ALL.len(),
        "lease_and_fence.members must not list codes outside src/error_codes.rs::ALL"
    );
}

/// Event kinds the store emits at `emit_event` call sites with a string literal,
/// plus kinds only produced through documented dynamic sites below.
fn emitted_event_kinds() -> std::collections::BTreeSet<String> {
    fn find_call_end(text: &str, open_paren: usize) -> usize {
        let bytes = text.as_bytes();
        let mut depth = 0i32;
        let mut i = open_paren;
        while i < bytes.len() {
            match bytes[i] {
                b'"' => {
                    i += 1;
                    while i < bytes.len() && bytes[i] != b'"' {
                        i += if bytes[i] == b'\\' { 2 } else { 1 };
                    }
                }
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return i + 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        text.len()
    }

    fn split_top_level_args(args: &str) -> Vec<&str> {
        let bytes = args.as_bytes();
        let mut depth = 0i32;
        let mut start = 0usize;
        let mut out = Vec::new();
        let mut i = 0usize;
        while i < bytes.len() {
            match bytes[i] {
                b'"' => {
                    i += 1;
                    while i < bytes.len() && bytes[i] != b'"' {
                        i += if bytes[i] == b'\\' { 2 } else { 1 };
                    }
                }
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                b',' if depth == 0 => {
                    out.push(args[start..i].trim());
                    start = i + 1;
                }
                _ => {}
            }
            i += 1;
        }
        if start < args.len() {
            out.push(args[start..].trim());
        }
        out
    }

    let mut kinds = std::collections::BTreeSet::new();
    for (_, text) in rust_sources() {
        let mut from = 0usize;
        while let Some(hit) = text[from..].find("emit_event(") {
            let start = from + hit;
            let open_paren = start + "emit_event".len();
            let end = find_call_end(&text, open_paren);
            let args = split_top_level_args(&text[open_paren + 1..end - 1]);
            if let Some(kind_expr) = args.get(4) {
                if let Some(lit) = plain_string_literal(kind_expr) {
                    if lit != "system" {
                        kinds.insert(lit.to_string());
                    }
                }
            }
            from = end;
        }
    }
    for k in [
        "question_followup_requested",
        "question_replied",
        "schedule_proposed",
        "schedule_created",
        "schedule_activated",
        "schedule_rejected",
        "schedule_paused",
        "schedule_updated",
        "user_disabled",
        "user_enabled",
    ] {
        kinds.insert(k.to_string());
    }
    kinds
}

/// Kinds listed in `x-event-kinds-reserved` — on the Event.kind enum but
/// deliberately never emitted.
fn spec_reserved_event_kinds() -> std::collections::BTreeMap<String, String> {
    let spec: Value = serde_norway::from_str(include_str!("../spec/openapi.yaml"))
        .expect("spec/openapi.yaml parses as YAML");
    let reserved = object_keys(
        &spec["x-event-kinds-reserved"],
        "spec/openapi.yaml x-event-kinds-reserved",
    );
    let mut out = std::collections::BTreeMap::new();
    for kind in reserved {
        let description = spec["x-event-kinds-reserved"][&kind]
            .as_str()
            .unwrap_or_default();
        assert!(
            description.len() > 40,
            "`x-event-kinds-reserved.{kind}` must say why the kind is reserved \
             and never emitted — a bare label documents nothing"
        );
        assert!(
            description.contains("never emitted") || description.contains("Never emitted"),
            "`x-event-kinds-reserved.{kind}` must state explicitly that the kind \
             is never emitted"
        );
        out.insert(kind, description.to_string());
    }
    out
}

fn spec_event_kinds() -> std::collections::BTreeSet<String> {
    let spec: Value = serde_norway::from_str(include_str!("../spec/openapi.yaml"))
        .expect("spec/openapi.yaml parses as YAML");
    spec["components"]["schemas"]["Event"]["properties"]["kind"]["enum"]
        .as_array()
        .expect("Event.kind enum")
        .iter()
        .map(|v| v.as_str().expect("kind").to_string())
        .collect()
}

#[test]
fn every_emitted_event_kind_is_listed_on_event_kind_enum() {
    let emitted = emitted_event_kinds();
    let documented = spec_event_kinds();
    let reserved = spec_reserved_event_kinds();

    let missing: Vec<String> = emitted
        .iter()
        .filter(|k| !documented.contains(*k))
        .cloned()
        .collect();
    assert!(
        missing.is_empty(),
        "these event kinds are emitted under src/ but missing from Event.kind \
         enum in spec/openapi.yaml:\n  {}\n\nThe event log is the integration \
         surface — add each kind to the enum.",
        missing.join("\n  ")
    );

    let stale: Vec<String> = documented
        .iter()
        .filter(|k| !emitted.contains(k.as_str()) && !reserved.contains_key(k.as_str()))
        .cloned()
        .collect();
    assert!(
        stale.is_empty(),
        "Event.kind enum lists kinds neither emitted nor reserved in \
         x-event-kinds-reserved:\n  {}\n\nEmit them, reserve them with a reason, \
         or remove the enum entry.",
        stale.join("\n  ")
    );

    for (kind, reason) in &reserved {
        assert!(
            documented.contains(kind.as_str()),
            "x-event-kinds-reserved lists `{kind}` but Event.kind enum does not"
        );
        assert!(
            !emitted.contains(kind.as_str()),
            "`{kind}` is reserved as never emitted ({reason}) but src/ emits it"
        );
    }
}

// ---------------------------------------------------------------------------
// Route ↔ spec path bijection (takomo-4tn3).
// ---------------------------------------------------------------------------

/// OpenAPI documents some SPA shells under `-page` keys because `/v1/foo` and
/// `/foo` cannot share one YAML key. Each alias maps to the path the router serves.
const SPEC_PATH_ALIASES: &[(&str, &str)] = &[
    ("/schedules-page", "/schedules"),
    ("/mindmaps-page", "/mindmaps"),
    ("/inbox-page", "/inbox"),
    ("/initiatives-page", "/initiatives"),
    ("/settings-page", "/settings"),
];

/// Router-only paths documented under another spec key.
const ROUTER_PATH_ALIASES: &[(&str, &str)] = &[(
    "/.well-known/oauth-protected-resource/mcp",
    "/.well-known/oauth-protected-resource",
)];

fn router_paths_from_server_rs() -> std::collections::BTreeSet<String> {
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/server.rs"),
    )
    .expect("read src/server.rs");
    let mut paths = std::collections::BTreeSet::new();
    for chunk in text.split(".route(").skip(1) {
        let trimmed = chunk.trim_start();
        if let Some(inner) = trimmed.strip_prefix('"') {
            if let Some(end) = inner.find('"') {
                paths.insert(inner[..end].to_string());
            }
        }
    }
    paths.insert("/mcp".to_string());
    paths
}

fn spec_path_is_root_scoped(path_item: &Value) -> bool {
    path_item
        .get("servers")
        .and_then(|s| s.as_array())
        .and_then(|a| a.first())
        .and_then(|e| e.get("url"))
        .and_then(|u| u.as_str())
        == Some("/")
}

fn resolve_spec_path_alias(spec_path: &str) -> &str {
    SPEC_PATH_ALIASES
        .iter()
        .find(|(from, _)| *from == spec_path)
        .map(|(_, to)| *to)
        .unwrap_or(spec_path)
}

fn spec_paths_as_router_paths() -> std::collections::BTreeSet<String> {
    let spec: Value = serde_norway::from_str(include_str!("../spec/openapi.yaml"))
        .expect("spec/openapi.yaml parses as YAML");
    let mut out = std::collections::BTreeSet::new();
    for (path, item) in spec["paths"].as_object().expect("paths") {
        let resolved = resolve_spec_path_alias(path);
        let router = if spec_path_is_root_scoped(item) {
            resolved.to_string()
        } else {
            format!("/v1{resolved}")
        };
        out.insert(router);
    }
    out
}

fn normalize_router_path(path: &str) -> &str {
    ROUTER_PATH_ALIASES
        .iter()
        .find(|(from, _)| *from == path)
        .map(|(_, to)| *to)
        .unwrap_or(path)
}

#[test]
fn every_router_path_appears_in_the_spec_and_vice_versa() {
    let router: std::collections::BTreeSet<String> = router_paths_from_server_rs()
        .into_iter()
        .map(|p| normalize_router_path(&p).to_string())
        .collect();
    let spec = spec_paths_as_router_paths();

    let only_router: Vec<&str> = router
        .iter()
        .map(String::as_str)
        .filter(|p| !spec.contains(*p))
        .collect();
    assert!(
        only_router.is_empty(),
        "these routes are registered in src/server.rs (or /mcp) but have no \
         matching path in spec/openapi.yaml:\n  {}\n\nAdd the path to the spec, \
         or a ROUTER_PATH_ALIASES entry if it is documented under another key.",
        only_router.join("\n  ")
    );

    let only_spec: Vec<&str> = spec
        .iter()
        .map(String::as_str)
        .filter(|p| !router.contains(*p))
        .collect();
    assert!(
        only_spec.is_empty(),
        "these paths are in spec/openapi.yaml but not registered in the router:\n  \
         {}\n\nRemove stale spec entries or register the route.",
        only_spec.join("\n  ")
    );
}

#[tokio::test]
async fn patch_rejects_state_and_unknown_fields() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Patch teaching test").await;

    let (s, body) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "state": "done" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(body["code"], "patch.state_not_patchable");
    assert!(body["remedy"].as_str().unwrap().contains("/transition"));

    // metadata_merge with RFC 7386 delete semantics.
    let (s, _) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "metadata_merge": { "test.keep": "yes", "test.drop": "tmp" } }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, body) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "metadata_merge": { "test.drop": null } }),
        )
        .await;
    assert_eq!(body["metadata"]["test.keep"], "yes");
    assert!(body["metadata"].get("test.drop").is_none());
}

#[tokio::test]
async fn non_holder_writes_restricted_while_claimed() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Claimed-ticket write restrictions").await;
    app.to_ready(&id).await;
    app.claim(&id).await;

    // Non-holder title patch: 409 claim.held.
    let (s, body) = app
        .patch(
            &app.worker2,
            &format!("/v1/tickets/{id}"),
            json!({ "title": "stolen" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(body["code"], "claim.held");

    // Non-holder may merge metadata under its own namespace.
    let (s, _) = app
        .patch(
            &app.worker2,
            &format!("/v1/tickets/{id}"),
            json!({ "metadata_merge": { "agent:w2.note": "observed" } }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);

    // ...but not under someone else's namespace.
    let (s, _) = app
        .patch(
            &app.worker2,
            &format!("/v1/tickets/{id}"),
            json!({ "metadata_merge": { "agent:w1.note": "forged" } }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);

    // Comments stay open to everyone.
    let (s, _) = app
        .post(
            &app.worker2,
            &format!("/v1/tickets/{id}/comments"),
            json!({ "body": "fyi" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);

    // Holder must echo the fence even for its own patches.
    let (s, body) = app
        .patch(
            &app.worker,
            &format!("/v1/tickets/{id}"),
            json!({ "title": "renamed by holder" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(body["code"], "fence.required");
}

#[tokio::test]
async fn sse_stream_delivers_events() {
    let app = TestApp::spawn().await;
    app.create_ticket("SSE seed ticket").await;

    let resp = app
        .authed(Method::GET, &app.admin, "/v1/events/stream?since=0")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("text/event-stream"));

    use futures::StreamExt;
    let mut stream = resp.bytes_stream();
    let chunk = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("SSE first chunk within 5s")
        .expect("stream item")
        .expect("bytes");
    let text = String::from_utf8_lossy(&chunk);
    assert!(
        text.contains("id:"),
        "SSE frame should carry seq ids: {text}"
    );
    assert!(
        text.contains("created"),
        "SSE frame should carry the created event: {text}"
    );
}

#[tokio::test]
async fn project_scoped_token_is_fenced_in() {
    let app = TestApp::spawn().await;
    app.open_store()
        .create_project("other", "Other Project", None, "test:setup")
        .unwrap();
    let scoped = app.mint("agent:tp-only", &["read", "write"], Some(&["tp"]));

    let (s, body) = app
        .post(
            &scoped,
            "/v1/tickets",
            json!({ "project": "other", "title": "Reach across" }),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "auth.project");

    let (s, _) = app
        .post(
            &scoped,
            "/v1/tickets",
            json!({ "project": "tp", "title": "Stay inside" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
}

#[tokio::test]
async fn stale_fence_bounces_even_on_unclaimed_ticket() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Zombie fence on unclaimed ticket").await;
    app.to_ready(&id).await;

    // Claim/release twice: fence advances to 2, ticket ends unclaimed.
    for expected_fence in 1..=2i64 {
        let fence = app.claim(&id).await;
        assert_eq!(fence, expected_fence);
        let (s, _) = app
            .post(
                &app.worker,
                &format!("/v1/tickets/{id}/release"),
                json!({ "fence": fence }),
            )
            .await;
        assert_eq!(s, StatusCode::NO_CONTENT);
    }

    // A zombie echoing fence 1 must bounce on PATCH even though the ticket is
    // unclaimed now.
    let (s, body) = app
        .patch(
            &app.worker,
            &format!("/v1/tickets/{id}"),
            json!({ "title": "zombie write", "fence": 1 }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(body["code"], "fence.stale");

    // Same on transition.
    let (s, body) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/transition"),
            json!({ "to": "cancelled", "fence": 1 }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "fence.stale");

    // The current fence (2) is accepted on an unclaimed ticket.
    let (s, _) = app
        .patch(
            &app.worker,
            &format!("/v1/tickets/{id}"),
            json!({ "title": "current fence ok", "fence": 2 }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
}

// takomo-cjel: admin force-release. `POST /release` answers only to the holder
// and the sweeper frees a lease exactly when it expires, so before this route a
// ticket held by a crashed worker had no API recovery at all — and since
// `max_claim_ttl_seconds` has no ceiling, "when it expires" is however long the
// project says.
#[tokio::test]
async fn admin_force_release_ousts_the_holder_and_bumps_the_fence() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Held by a worker that is gone").await;
    app.to_ready(&id).await;
    let stale_fence = app.claim(&id).await;

    let (s, forced) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{id}/force-release"),
            json!({ "reason": "agent:w1 crashed; 4h lease" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "force-release failed: {forced}");
    assert_eq!(forced["ticket"], json!(id));
    assert_eq!(forced["project"], json!("tp"));
    assert_eq!(forced["previous_holder"], json!("agent:w1"));
    assert_eq!(forced["previous_fence"], json!(stale_fence));
    assert_eq!(
        forced["fence"],
        json!(stale_fence + 1),
        "the fence must advance, or the displaced worker keeps writing: {forced}"
    );
    assert_eq!(forced["lease_expired"], json!(false));
    assert_eq!(forced["reason"], json!("agent:w1 crashed; 4h lease"));

    // The claim is gone.
    let (_, ticket) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
    assert_eq!(
        ticket["claim"],
        Value::Null,
        "claim must be cleared: {ticket}"
    );

    // The whole point: the displaced worker may still be alive, and every route
    // it could write through now refuses its fence.
    let (s, body) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/heartbeat"),
            json!({ "fence": stale_fence }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "fence.stale");

    let (s, body) = app
        .patch(
            &app.worker,
            &format!("/v1/tickets/{id}"),
            json!({ "title": "zombie write", "fence": stale_fence }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "fence.stale");

    let (s, body) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/transition"),
            json!({ "to": "implementing", "fence": stale_fence }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "fence.stale");

    // And the ticket is genuinely recoverable: someone else can claim it.
    let new_fence = app.claim_as(&app.worker2, &id).await;
    assert!(
        new_fence > forced["fence"].as_i64().unwrap(),
        "reclaim must advance the fence again"
    );

    // The audit trail is its own event kind, attributed to the admin, naming
    // both fences and the reason — not a `released` event that would read as the
    // holder letting go voluntarily.
    let (_, revoked) = app
        .get(
            &app.admin,
            &format!("/v1/events?since=0&ticket={id}&kind=lease_revoked"),
        )
        .await;
    let events = revoked["events"].as_array().expect("events array");
    assert_eq!(
        events.len(),
        1,
        "one lease_revoked event expected: {revoked}"
    );
    let e = &events[0];
    assert_eq!(e["actor"], json!("human:admin"));
    assert_eq!(e["payload"]["holder"], json!("agent:w1"));
    assert_eq!(e["payload"]["fence"], json!(stale_fence));
    assert_eq!(e["payload"]["new_fence"], json!(stale_fence + 1));
    assert_eq!(e["payload"]["lease_expired"], json!(false));
    assert_eq!(e["payload"]["reason"], json!("agent:w1 crashed; 4h lease"));

    let (_, released) = app
        .get(
            &app.admin,
            &format!("/v1/events?since=0&ticket={id}&kind=released"),
        )
        .await;
    assert!(
        released["events"]
            .as_array()
            .expect("events array")
            .is_empty(),
        "a forced release must not masquerade as a voluntary `released`: {released}"
    );
}

/// A separate route rather than a `force` flag on `/release`, so no ordinary
/// worker's typo can become a force: the `admin` scope AND the distinct path.
#[tokio::test]
async fn force_release_is_admin_only_and_accepts_no_fence() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Not yours to take").await;
    app.to_ready(&id).await;
    let fence = app.claim(&id).await;

    for (who, token) in [("the holder", &app.worker), ("a human", &app.human)] {
        let (s, body) = app
            .post(token, &format!("/v1/tickets/{id}/force-release"), json!({}))
            .await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{who} must be refused: {body}");
        assert_eq!(body["code"], "auth.scope");
    }

    // …and the refusal left the lease exactly where it was.
    let (_, ticket) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
    assert_eq!(ticket["claim"]["holder"], json!("agent:w1"), "{ticket}");

    // No `fence` field exists to get wrong — the caller is displacing a holder
    // whose fence it has no way to know.
    let (s, body) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{id}/force-release"),
            json!({ "fence": fence }),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "validation.unknown_field");

    // A bodiless POST is fine: `reason` is optional.
    let (s, forced) = app
        .json(app.authed(
            Method::POST,
            &app.admin,
            &format!("/v1/tickets/{id}/force-release"),
        ))
        .await;
    assert_eq!(s, StatusCode::OK, "a reasonless force-release: {forced}");
    assert_eq!(forced["reason"], Value::Null);
    assert_eq!(forced["previous_fence"], json!(fence));
}

#[tokio::test]
async fn force_release_without_a_claim_teaches_instead_of_lying() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Nothing to take").await;
    app.to_ready(&id).await;

    let (s, body) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{id}/force-release"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "claim.none");
    assert!(
        body["remedy"].as_str().is_some_and(|r| !r.is_empty()),
        "errors carry a remedy: {body}"
    );
    assert_eq!(body["current_state"], json!("ready"));

    // Not idempotent bookkeeping: it reports what it displaced, so the second
    // call on the same ticket lands in the same 409.
    app.claim(&id).await;
    let (s, _) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{id}/force-release"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, body) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{id}/force-release"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "claim.none");

    let (s, body) = app
        .post(&app.admin, "/v1/tickets/tp-nope/force-release", json!({}))
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["code"], "notfound.ticket");
}

/// An already-lapsed lease is force-released too rather than refused. The holder
/// is still recorded on the row, and natural expiry does *not* bump the fence,
/// so a zombie echoing that fence is still accepted until something does. Were
/// this a 409, the outcome would depend on whether the TTL happened to elapse a
/// moment before the admin's call.
#[tokio::test]
async fn force_release_also_fences_off_an_expired_holder() {
    // No sweeper: otherwise whether the lapsed claim is still on the row when
    // the force lands is a race against a 250ms timer.
    let app = TestApp::spawn_without_sweeper().await;
    let id = app
        .create_ticket("Lease lapsed, holder still recorded")
        .await;
    app.to_ready(&id).await;
    let stale_fence = app.claim_ttl(&app.worker, &id, Some(1)).await;
    tokio::time::sleep(Duration::from_millis(1200)).await;

    let (s, forced) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{id}/force-release"),
            json!({ "reason": "reclaiming a lapsed lease" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "force-release failed: {forced}");
    assert_eq!(forced["previous_holder"], json!("agent:w1"));
    assert_eq!(forced["lease_expired"], json!(true), "{forced}");
    assert_eq!(forced["fence"], json!(stale_fence + 1));

    // One event, not two: the force does not also log a `lease_expired`.
    let (_, expired) = app
        .get(
            &app.admin,
            &format!("/v1/events?since=0&ticket={id}&kind=lease_expired"),
        )
        .await;
    assert!(
        expired["events"]
            .as_array()
            .expect("events array")
            .is_empty(),
        "the force is the event, not a synthetic expiry too: {expired}"
    );
    let (_, revoked) = app
        .get(
            &app.admin,
            &format!("/v1/events?since=0&ticket={id}&kind=lease_revoked"),
        )
        .await;
    assert_eq!(
        revoked["events"].as_array().expect("events array").len(),
        1,
        "{revoked}"
    );
    assert_eq!(
        revoked["events"][0]["payload"]["lease_expired"],
        json!(true)
    );

    // The fence bump is what closes the door on the ousted worker; expiry alone
    // would have left its fence current.
    let (s, body) = app
        .patch(
            &app.worker,
            &format!("/v1/tickets/{id}"),
            json!({ "title": "zombie write", "fence": stale_fence }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "fence.stale");
}

/// A lapsed lease stays force-releasable after the sweeper has cleared the claim
/// row, because what is left is not nothing: it is the lapsed holder's permission
/// to resume the lease in place (takomo-jb5i). If the force stopped at
/// `claim.none` there, an operator reassigning an in-flight ticket would have no
/// way to stop the previous worker from simply taking it back.
#[tokio::test]
async fn force_release_ends_a_lapsed_holders_right_to_resume() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Reassigned out from under a lapse").await;
    let stale_fence = lapse_mid_implementation(&app, &id).await;

    let (s, forced) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{id}/force-release"),
            json!({ "reason": "reassigning the work" }),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::OK,
        "a swept-away lapse is still force-releasable: {forced}"
    );
    assert_eq!(forced["previous_holder"], json!("agent:w1"));
    assert_eq!(forced["lease_expired"], json!(true), "{forced}");
    assert_eq!(forced["fence"], json!(stale_fence + 1));

    // The point of the call: the resume is gone.
    let (s, refused) = app
        .post(&app.worker, &format!("/v1/tickets/{id}/claim"), json!({}))
        .await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "the displaced holder must not resume: {refused}"
    );
    assert_eq!(refused["code"], "claim.state");
    assert_eq!(refused["details"]["lapsed_holder"], Value::Null);

    // And with nothing left to displace it is a `claim.none` like any other.
    let (s, none) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{id}/force-release"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{none}");
    assert_eq!(none["code"], "claim.none");
}

#[tokio::test]
async fn deps_respect_claims_and_fences() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Deps under claim").await;
    let other = app.create_ticket("A blocker candidate").await;
    app.to_ready(&id).await;
    let fence = app.claim(&id).await;

    // Non-holder cannot add a dep to a claimed ticket.
    let (s, body) = app
        .post(
            &app.worker2,
            &format!("/v1/tickets/{id}/deps"),
            json!({ "blocked_by": other }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "claim.held");

    // Holder without the fence is refused too.
    let (s, body) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/deps"),
            json!({ "blocked_by": other }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "fence.required");

    // Holder with the fence succeeds, and the ticket version bumps.
    let (_, before) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
    let v_before = before["version"].as_i64().unwrap();
    let (s, body) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/deps"),
            json!({ "blocked_by": other, "fence": fence }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let (_, after) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
    assert!(after["version"].as_i64().unwrap() > v_before);
    assert_eq!(after["blocked_by"][0], other.as_str());

    // Removal follows the same rule (fence via query param).
    let (s, _) = app
        .delete(
            &app.worker,
            &format!("/v1/tickets/{id}/deps?blocked_by={other}"),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "holder needs fence on delete too");
    let (s, _) = app
        .delete(
            &app.worker,
            &format!("/v1/tickets/{id}/deps?blocked_by={other}&fence={fence}"),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn autoland_twin_edge_selects_most_actionable_error() {
    let app = TestApp::spawn().await;
    // Second project with a twin review->done edge: human gate OR autoland gate.
    let (s, body) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({
                "id": "auto",
                "name": "Autoland",
                "workflow": {
                    "name": "auto-wf",
                    "initial": "ready",
                    "states": [
                        { "id": "ready", "category": "todo", "claimable": true },
                        { "id": "review", "category": "review" },
                        { "id": "done", "category": "done", "terminal": true },
                        { "id": "cancelled", "category": "cancelled", "terminal": true }
                    ],
                    "transitions": [
                        { "from": "ready", "to": "review" },
                        { "from": "ready", "to": "cancelled" },
                        { "from": "review", "to": "done", "requires": ["scope:human", "guard:no_open_children"] },
                        { "from": "review", "to": "done", "requires": ["scope:autoland", "guard:no_open_children"] },
                        { "from": "review", "to": "cancelled", "requires": ["scope:human"] }
                    ]
                }
            }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");

    let orch = app.mint("orch:main", &["read", "write", "autoland"], None);

    let (s, t) = app
        .post(
            &orch,
            "/v1/tickets",
            json!({ "project": "auto", "title": "Autoland candidate" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let id = t["id"].as_str().unwrap().to_string();
    let (s, child) = app
        .post(
            &orch,
            "/v1/tickets",
            json!({ "project": "auto", "title": "Open child", "parent": id }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let child_id = child["id"].as_str().unwrap().to_string();
    let (s, b) = app.transition(&orch, &id, "review").await;
    assert_eq!(s, StatusCode::OK, "{b}");

    // Autoland token + open child: the autoland edge is authorized, so the
    // most actionable failure is the guard 409 — not a 403.
    let (s, body) = app.transition(&orch, &id, "done").await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "transition.guard");
    assert_eq!(body["details"]["offending_tickets"][0], child_id.as_str());

    // A token with neither scope gets the 403.
    let (s, body) = app.transition(&app.worker, &id, "done").await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "transition.scope");

    // Close the child; autoland lands it without any human scope.
    let (s, b) = app.transition(&orch, &child_id, "cancelled").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, body) = app.transition(&orch, &id, "done").await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "done");
}

#[tokio::test]
async fn workflow_upload_rejects_typos_and_terminal_exits() {
    let app = TestApp::spawn().await;

    // Misspelled 'requires' must be a 422, not a silently dropped gate.
    let (s, body) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({
                "id": "typo",
                "name": "Typo",
                "workflow": {
                    "name": "typo-wf",
                    "initial": "open",
                    "states": [
                        { "id": "open", "category": "todo" },
                        { "id": "done", "category": "done", "terminal": true }
                    ],
                    "transitions": [
                        { "from": "open", "to": "done", "require": ["scope:human"] }
                    ]
                }
            }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "workflow.parse");

    // Outgoing transitions from terminal states are refused.
    let (s, body) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({
                "id": "reopen",
                "name": "Reopen",
                "workflow": {
                    "name": "reopen-wf",
                    "initial": "open",
                    "states": [
                        { "id": "open", "category": "todo" },
                        { "id": "done", "category": "done", "terminal": true }
                    ],
                    "transitions": [
                        { "from": "open", "to": "done" },
                        { "from": "done", "to": "open" }
                    ]
                }
            }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "workflow.invalid");
    assert!(body["message"].as_str().unwrap().contains("terminal"));
}

// Pilot finding A: human approval is authoritative over a held claim. A
// `scope:human` transition performed by a human-scoped caller must succeed even
// while a different actor holds the lease, and must auto-release that lease.
#[tokio::test]
async fn human_transition_overrides_held_claim_and_auto_releases() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Human gate over a worker's lease").await;

    // Move it into `spec` (claimable) and let the worker take the lease.
    let (s, b) = app.transition(&app.worker, &id, "spec").await;
    assert_eq!(s, StatusCode::OK, "brief->spec: {b}");
    let (s, lease) = app
        .post(&app.worker, &format!("/v1/tickets/{id}/claim"), json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "worker claim: {lease}");
    assert_eq!(lease["holder"], "agent:w1");

    // The human (a DIFFERENT actor) approves spec->ready — a scope:human edge —
    // while the worker still holds the lease. It must succeed, not 409.
    let (s, body) = app.transition(&app.human, &id, "ready").await;
    assert_eq!(
        s,
        StatusCode::OK,
        "human override should win over the lease: {body}"
    );
    assert_eq!(body["state"], "ready");
    // The lease is gone: the human transition superseded and released it.
    assert!(
        body["claim"].is_null(),
        "claim must be auto-released: {body}"
    );

    // Both a `transitioned` and a `released` event landed, attributed to the
    // human, with the superseding reason on the release.
    let (_, trans) = app
        .get(
            &app.admin,
            &format!("/v1/events?since=0&ticket={id}&kind=transitioned"),
        )
        .await;
    let tev = trans["events"].as_array().unwrap();
    let approve = tev
        .iter()
        .find(|e| e["payload"]["to"] == "ready")
        .expect("spec->ready transitioned event");
    assert_eq!(approve["actor"], "human:reviewer");
    assert_eq!(approve["payload"]["auto_released"], true);

    let (_, rel) = app
        .get(
            &app.admin,
            &format!("/v1/events?since=0&ticket={id}&kind=released"),
        )
        .await;
    let rev = rel["events"].as_array().unwrap();
    let superseded = rev
        .iter()
        .find(|e| e["payload"]["reason"] == "superseded by human transition")
        .expect("a `released` event superseding the worker's lease");
    assert_eq!(superseded["actor"], "human:reviewer");
}

// The holder lock is unchanged for ordinary `claim`-required transitions: a
// non-holder without the human scope is still blocked (finding A is scoped to
// human-required edges only).
#[tokio::test]
async fn holder_lock_still_blocks_non_holder_ordinary_transition() {
    let app = TestApp::spawn().await;
    let id = app
        .create_ticket("Ordinary claim edge keeps holder lock")
        .await;
    app.to_ready(&id).await;

    // Worker 1 holds the lease.
    app.claim(&id).await;

    // Worker 2 (non-holder, no human scope) attempts ready->implementing, an
    // ordinary `claim`-required edge. The holder lock still blocks it.
    let (s, body) = app.transition(&app.worker2, &id, "implementing").await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "claim.held");
}

// Pilot finding B: validation order is legality -> scope -> claim/fence, so the
// headline error names the first real blocker rather than a fencing complaint.
#[tokio::test]
async fn error_ordering_legality_and_scope_precede_fence() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Error ordering over a held lease").await;
    let (s, b) = app.transition(&app.worker, &id, "spec").await;
    assert_eq!(s, StatusCode::OK, "brief->spec: {b}");
    // Worker holds the lease but echoes NO fence on the attempts below; before
    // the fix both would have surfaced `fence.required`.
    app.claim(&id).await;

    // (scope before fence) The worker attempts spec->ready, a human gate it
    // lacks the scope for: a 403 transition.scope, not fence.required.
    let (s, body) = app.transition(&app.worker, &id, "ready").await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "transition.scope");
    assert!(body["message"].as_str().unwrap().contains("human"));

    // (legality before fence) The worker attempts an undefined spec->done edge:
    // transition.illegal, not fence.required.
    let (s, body) = app.transition(&app.worker, &id, "done").await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "transition.illegal");
    assert!(body["allowed_transitions"].is_array());
}

// --- token & identity over HTTP ---------------------------------------------

#[tokio::test]
async fn token_mint_list_revoke_and_whoami_over_http() {
    let app = TestApp::spawn().await;

    // Admin mints a project-scoped read,write token over HTTP.
    let (s, minted) = app
        .post(
            &app.admin,
            "/v1/tokens",
            json!({ "actor": "agent:http", "scopes": ["read", "write"], "projects": ["tp"] }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{minted}");
    let plaintext = minted["token"]
        .as_str()
        .expect("plaintext token shown once");
    assert!(
        plaintext.starts_with("tk_"),
        "token should be a tk_ plaintext: {minted}"
    );
    let token_id = minted["id"].as_str().expect("token id").to_string();
    assert_eq!(minted["actor"], "agent:http");
    assert_eq!(minted["scopes"], json!(["read", "write"]));
    assert_eq!(minted["projects"], json!(["tp"]));
    // The mint response must never leak the at-rest hash.
    assert!(
        minted.get("hash").is_none(),
        "hash must not be returned: {minted}"
    );

    // The freshly minted token authenticates and can create work in its project.
    let plaintext = plaintext.to_string();
    let (s, t) = app
        .post(
            &plaintext,
            "/v1/tickets",
            json!({ "project": "tp", "title": "Minted-token ticket" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "minted token should work: {t}");

    // whoami reflects the caller's identity (any valid token may call it).
    let (s, who) = app.get(&plaintext, "/v1/whoami").await;
    assert_eq!(s, StatusCode::OK, "{who}");
    assert_eq!(who["actor"], "agent:http");
    assert_eq!(who["scopes"], json!(["read", "write"]));
    assert_eq!(who["projects"], json!(["tp"]));

    // Admin lists tokens: metadata only — never plaintext or hash.
    let (s, rows) = app.get(&app.admin, "/v1/tokens").await;
    assert_eq!(s, StatusCode::OK);
    let rows = rows.as_array().expect("token list array");
    let row = rows
        .iter()
        .find(|r| r["id"] == token_id.as_str())
        .expect("minted token appears in the list");
    assert_eq!(row["actor"], "agent:http");
    assert!(row["revoked_at"].is_null(), "not yet revoked: {row}");
    for r in rows {
        assert!(
            r.get("token").is_none(),
            "list must not leak plaintext: {r}"
        );
        assert!(r.get("hash").is_none(), "list must not leak the hash: {r}");
    }

    // Admin revokes it; the minted token then fails auth.
    let (s, _) = app
        .delete(&app.admin, &format!("/v1/tokens/{token_id}"))
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    let (s, body) = app.get(&plaintext, "/v1/whoami").await;
    assert_eq!(
        s,
        StatusCode::UNAUTHORIZED,
        "revoked token must fail: {body}"
    );
    assert_eq!(body["code"], "auth.invalid");

    // Revoking an unknown id is a teaching 404.
    let (s, _) = app.delete(&app.admin, "/v1/tokens/tok_nope").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn token_admin_endpoints_require_admin_scope() {
    let app = TestApp::spawn().await;

    // A non-admin (read,write) token is 403'd on every token-admin endpoint.
    let (s, body) = app
        .post(
            &app.worker,
            "/v1/tokens",
            json!({ "actor": "agent:x", "scopes": ["read"] }),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "auth.scope");

    let (s, body) = app.get(&app.worker, "/v1/tokens").await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "auth.scope");

    let (s, _) = app.delete(&app.worker, "/v1/tokens/tok_whatever").await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // But whoami is open to any valid token, and admin sees projects "*".
    let (s, who) = app.get(&app.worker, "/v1/whoami").await;
    assert_eq!(s, StatusCode::OK, "{who}");
    assert_eq!(who["actor"], "agent:w1");
    let (s, admin_who) = app.get(&app.admin, "/v1/whoami").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(admin_who["projects"], "*");
    assert!(admin_who["scopes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "admin"));
}

#[tokio::test]
async fn token_create_validates_body() {
    let app = TestApp::spawn().await;

    // Missing scopes.
    let (s, body) = app
        .post(&app.admin, "/v1/tokens", json!({ "actor": "agent:y" }))
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "token.scopes");

    // "*" projects means all projects (None internally).
    let (s, minted) = app
        .post(
            &app.admin,
            "/v1/tokens",
            json!({ "actor": "orch:all", "scopes": ["read", "write", "admin"], "projects": "*",
                    "expires_seconds": 3600 }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{minted}");
    assert_eq!(minted["projects"], "*");
    assert!(minted["expires_at"].is_string(), "expiry echoed: {minted}");

    // Empty projects array is rejected (use "*" for all).
    let (s, body) = app
        .post(
            &app.admin,
            "/v1/tokens",
            json!({ "actor": "agent:z", "scopes": ["read"], "projects": [] }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "token.projects");
}

// --- Tier 3 DX polish -------------------------------------------------------

// similar[] scores by title-token overlap (Jaccard) plus a type-match nudge,
// thresholded so a real near-duplicate surfaces with its score and matched
// terms, while an incidental single shared word does not cry wolf.
#[tokio::test]
async fn similar_is_scored_and_thresholded() {
    let app = TestApp::spawn().await;
    let base = app.create_ticket("Optimize the database indexes").await;
    // Incidental single-word overlap ("database") — must NOT surface later.
    app.create_ticket("Database migration tooling script").await;

    let (s, body) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "tp", "title": "Optimize the database indexes for reads" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let similar = body["similar"].as_array().expect("similar array");

    // Exactly the genuine near-duplicate surfaces.
    assert_eq!(
        similar.len(),
        1,
        "only the real dupe should surface: {body}"
    );
    let hit = &similar[0];
    assert_eq!(hit["id"], base.as_str());
    let score = hit["score"].as_f64().expect("numeric score");
    assert!(score > 0.5, "near-duplicate should score high, got {score}");
    let terms: Vec<&str> = hit["matched_terms"]
        .as_array()
        .expect("matched_terms array")
        .iter()
        .map(|t| t.as_str().unwrap())
        .collect();
    assert!(terms.contains(&"database"), "matched_terms: {terms:?}");
    assert!(terms.contains(&"optimize"), "matched_terms: {terms:?}");
    assert!(hit["type"].is_string(), "type echoed: {hit}");
}

// A fence greater than the current one was never issued -> fence.invalid (a
// client bug), distinct from a stale (superseded, lower) fence.
#[tokio::test]
async fn fence_greater_than_current_is_invalid_not_stale() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Fence-never-issued test").await;
    app.to_ready(&id).await;

    // Claim: fence becomes 1.
    let fence = app.claim(&id).await;
    assert_eq!(fence, 1);

    // Holder echoes a fence the store never issued (fabricated, too high).
    let (s, body) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/heartbeat"),
            json!({ "fence": fence + 5 }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "fence.invalid", "{body}");
    assert!(body["message"].as_str().unwrap().contains("never issued"));
    assert_eq!(body["details"]["current_fence"], fence);

    // The correct current fence still works.
    let (s, ok) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/heartbeat"),
            json!({ "fence": fence }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{ok}");
}

// PATCH links merges per key instead of replacing the whole object; a null
// value deletes just that key.
#[tokio::test]
async fn links_patch_merges_per_key() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Links merge test").await;

    let (s, b) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "links": { "branch": "feat/x" } }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["links"]["branch"], "feat/x");

    // Add a second key — the first must survive (not be clobbered).
    let (s, b) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "links": { "pr": "https://example.test/pr/1" } }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(
        b["links"]["branch"], "feat/x",
        "existing key must persist: {b}"
    );
    assert_eq!(b["links"]["pr"], "https://example.test/pr/1");

    // null deletes just that key.
    let (s, b) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "links": { "branch": null } }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(
        b["links"].get("branch").is_none(),
        "branch should be deleted: {b}"
    );
    assert_eq!(
        b["links"]["pr"], "https://example.test/pr/1",
        "pr should remain: {b}"
    );

    // Non-string, non-null value is rejected.
    let (s, b) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "links": { "pr": 5 } }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert_eq!(b["code"], "validation.links");
}

#[tokio::test]
async fn links_are_capped_like_other_free_form_fields() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Link cap test").await;

    let huge = "x".repeat(9_000);
    let (s, b) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "links": { "pr": huge } }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert_eq!(b["code"], "validation.links");
}

// GET /tickets?q= is tokenized: every term must match, across title OR body.
#[tokio::test]
async fn search_is_tokenized_all_terms_match() {
    let app = TestApp::spawn().await;
    // Title carries one term, body the other.
    let (s, _t) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "tp", "title": "Refactor the auth layer", "body": "replace the token cache" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    app.create_ticket("Unrelated docs cleanup").await;

    // Both terms present (one in title, one in body) -> match.
    let (_, list) = app
        .get(&app.admin, "/v1/tickets?project=tp&q=auth+token")
        .await;
    let items = list["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "both terms should match one ticket: {list}");
    assert!(items[0]["title"].as_str().unwrap().contains("auth"));

    // One term matches, the other does not -> no results (AND semantics).
    let (_, list) = app
        .get(&app.admin, "/v1/tickets?project=tp&q=auth+nonexistentword")
        .await;
    assert!(
        list["items"].as_array().unwrap().is_empty(),
        "unmatched term must exclude the row: {list}"
    );

    // Case-insensitive.
    let (_, list) = app.get(&app.admin, "/v1/tickets?project=tp&q=AUTH").await;
    assert_eq!(list["items"].as_array().unwrap().len(), 1, "{list}");
}

// GET /v1/export streams JSONL of tickets with their comments and deps, and the
// output round-trips (every line is a self-contained JSON ticket).
#[tokio::test]
async fn export_streams_jsonl_with_comments_and_deps() {
    let app = TestApp::spawn().await;
    let a = app.create_ticket("Exportable ticket A").await;
    let b = app.create_ticket("Exportable ticket B blocks A").await;
    // A blocked_by B.
    let (s, _) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{a}/deps"),
            json!({ "blocked_by": b }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    // A comment on A.
    let (s, _) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{a}/comments"),
            json!({ "body": "a note" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);

    let (status, ctype, text) = app.get_raw(&app.admin, "/v1/export?project=tp").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        ctype.starts_with("application/x-ndjson"),
        "content-type: {ctype}"
    );

    let lines: Vec<Value> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("each line is valid JSON"))
        .collect();
    assert_eq!(lines.len(), 2, "two tickets exported: {text}");

    let line_a = lines
        .iter()
        .find(|l| l["id"] == a.as_str())
        .expect("A present");
    assert_eq!(
        line_a["blocked_by"][0],
        b.as_str(),
        "deps in export: {line_a}"
    );
    let comments = line_a["comments"].as_array().expect("comments array");
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0]["body"], "a note");

    // A read-only (no write) token can export; project scoping is honored.
    let reader = app.mint("agent:reader", &["read"], Some(&["tp"]));
    let (status, _c, text) = app.get_raw(&reader, "/v1/export?project=tp").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(text.lines().filter(|l| !l.trim().is_empty()).count(), 2);
}

// GET /v1/export/sqlite hands back the whole database as one openable SQLite
// file — and specifically one that carries data still sitting in the WAL.
//
// That last part is the reason the endpoint runs `VACUUM INTO` instead of
// copying the file: the server holds its writer connection open for its whole
// life, so nothing checkpoints, and a plain copy of `test.db` would be a torn
// snapshot missing every recent commit. The ticket below is created moments
// before the export, so if the snapshot were taken the naive way this assertion
// is what would fail.
#[tokio::test]
async fn sqlite_export_is_an_openable_snapshot_including_unflushed_wal() {
    let app = TestApp::spawn().await;
    let id = app
        .create_ticket("Ticket that must survive the snapshot")
        .await;

    let (status, disposition, bytes) = app
        .get_bytes(&app.admin, "/v1/export/sqlite", "content-disposition")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        disposition.starts_with("attachment; filename=\"takomo-"),
        "downloads as a named file: {disposition}"
    );
    assert!(
        bytes.starts_with(b"SQLite format 3\0"),
        "the body is a SQLite database, not an error document: {:?}",
        String::from_utf8_lossy(&bytes[..bytes.len().min(200)])
    );

    // Open the bytes as a real database and read the ticket back out of it.
    let restored = app.tmp.path().join("restored.db");
    std::fs::write(&restored, &bytes).expect("write snapshot");
    let conn = rusqlite::Connection::open(&restored).expect("snapshot opens");
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .expect("integrity_check runs");
    assert_eq!(integrity, "ok", "snapshot is not corrupt");
    let title: String = conn
        .query_row("SELECT title FROM tickets WHERE id = ?1", [&id], |r| {
            r.get(0)
        })
        .expect("the ticket is in the snapshot");
    assert_eq!(title, "Ticket that must survive the snapshot");

    // And the claim above is not decoration: the live `.db` file on its own does
    // NOT contain that title yet, because the commit is still in the `-wal`
    // sidecar. This is the assertion that would catch someone "simplifying"
    // snapshot_into into a std::fs::copy.
    let main_db = std::fs::read(app.db_path()).expect("read the live db file");
    let needle = b"Ticket that must survive the snapshot";
    assert!(
        !main_db
            .windows(needle.len())
            .any(|w| w == needle.as_slice()),
        "the commit is expected to still be in the WAL — if it has been checkpointed \
         into the main file this test no longer proves VACUUM INTO is required"
    );

    // The staging file is cleaned up: a snapshot is the size of the whole store,
    // so one leaked per download would fill the operator's disk.
    let leaked: Vec<_> = std::fs::read_dir(app.tmp.path())
        .expect("read tmp dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.contains("snapshot") || n.ends_with(".tmp"))
        .collect();
    assert!(leaked.is_empty(), "staging file left behind: {leaked:?}");
}

// `/settings` is the admin console, and the two halves of it that can break
// independently are asserted here: the route serves the app shell, and the
// bundle actually calls the endpoints the console is made of.
//
// The second half is what makes this more than a duplicate of the shared page
// test. A console that renders but calls nothing looks identical in a screenshot
// and is useless — and `/export/sqlite` in particular is reachable from no other
// surface, so nothing else would notice if it were dropped.
#[tokio::test]
async fn settings_page_serves_the_console_and_calls_the_admin_endpoints() {
    let app = TestApp::spawn().await;

    let resp = app.request(Method::GET, "/settings").send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_app_shell("/settings", &resp.text().await.unwrap());

    let bundle = app.eager_bundle().await;
    for path in ["/export/sqlite", "/tokens", "/projects"] {
        assert!(
            bundle.contains(path),
            "the settings console never calls {path} — the section is inert"
        );
    }
}

// The whole-database export needs `admin` AND a token with no project
// allowlist. The allowlist case is the one worth a test: the token has every
// scope, so only the explicit check stands between one project's admin and the
// other projects' rows, token hashes and OAuth client secrets.
#[tokio::test]
async fn sqlite_export_refuses_scoped_and_non_admin_tokens() {
    let app = TestApp::spawn().await;
    app.create_ticket("Present in the store").await;

    // Admin, but fenced to one project.
    let scoped = app.mint(
        "human:scoped-admin",
        &["read", "write", "human", "admin"],
        Some(&["tp"]),
    );
    let (status, _d, bytes) = app
        .get_bytes(&scoped, "/v1/export/sqlite", "content-type")
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a project-scoped admin cannot take the whole database"
    );
    let body: Value = serde_json::from_slice(&bytes).expect("error is JSON");
    assert_eq!(body["code"], "auth.project", "{body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("/v1/export?project="),
        "the error points at the export that IS project-shaped: {body}"
    );

    // Every other token kind is refused on the scope instead.
    for (label, token) in [("human", &app.human), ("worker", &app.worker)] {
        let (status, _d, bytes) = app
            .get_bytes(token, "/v1/export/sqlite", "content-type")
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{label} is refused");
        let body: Value = serde_json::from_slice(&bytes).expect("error is JSON");
        assert_eq!(body["code"], "auth.scope", "{label}: {body}");
    }
}

// GET /v1/metrics reports ticket counts by state and category per project, open
// claims, and the event total; a scoped token only sees its projects.
#[tokio::test]
async fn metrics_counts_by_state_category_and_claims() {
    let app = TestApp::spawn().await;
    let a = app.create_ticket("Metrics ticket one").await;
    app.create_ticket("Metrics ticket two").await;
    // Drive one into a claimed state.
    app.to_ready(&a).await;
    app.claim(&a).await;

    let (s, m) = app.get(&app.admin, "/v1/metrics").await;
    assert_eq!(s, StatusCode::OK, "{m}");
    let tp = &m["projects"]["tp"];
    assert_eq!(tp["total"], 2, "two tickets in tp: {m}");
    assert_eq!(tp["open_claims"], 1, "one open claim: {m}");
    // a was driven to `ready` (todo category) and claimed; the other is `brief`.
    assert_eq!(tp["by_state"]["brief"], 1, "{m}");
    assert_eq!(tp["by_state"]["ready"], 1, "{m}");
    // brief and ready are both `todo`-category in factory-default.
    assert_eq!(tp["by_category"]["todo"], 2, "{m}");
    assert_eq!(m["totals"]["tickets"], 2, "{m}");
    assert!(
        m["totals"]["events"].as_i64().unwrap() > 0,
        "events counted: {m}"
    );

    // A token scoped to a different project sees no tp counts.
    app.open_store()
        .create_project("solo", "Solo", None, "test:setup")
        .unwrap();
    let scoped = app.mint("agent:solo", &["read", "write"], Some(&["solo"]));
    let (s, m) = app.get(&scoped, "/v1/metrics").await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        m["projects"].get("tp").is_none(),
        "scoped token must not see tp: {m}"
    );
    assert_eq!(m["totals"]["tickets"], 0, "solo has no tickets yet: {m}");
}

// --- project archive --------------------------------------------------------

// The gate itself: while a project is archived every write under it is refused
// with a teaching 409, across every kind of write there is — and every read
// still answers. This is the test that fails if a new mutating route forgets
// `ensure_project_writable`.
#[tokio::test]
async fn project_archive_refuses_every_write_and_allows_every_read() {
    let app = TestApp::spawn().await;
    let existing = app.create_ticket("Written before the archive").await;
    let (s, _) = app
        .post(
            &app.admin,
            "/v1/projects/tp/tags",
            json!({ "kind": "person", "handle": "ada" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);

    let (s, body) = app
        .post(&app.admin, "/v1/projects/tp/archive", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["archived"], json!(true));
    assert!(body["archived_at"].is_string(), "{body}");

    // One write per surface that can reach a project's work.
    let (s, b) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "tp", "title": "after the archive" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "create ticket: {b}");
    assert_eq!(b["code"], "project.archived");
    assert_eq!(b["details"]["project"], json!("tp"));

    let (s, b) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{existing}/comments"),
            json!({ "body": "still here?" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "comment: {b}");
    assert_eq!(b["code"], "project.archived");

    let (s, b) = app.transition(&app.human, &existing, "spec").await;
    assert_eq!(s, StatusCode::CONFLICT, "transition: {b}");
    assert_eq!(b["code"], "project.archived");

    let (s, b) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{existing}"),
            json!({ "priority": "high" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "patch: {b}");
    assert_eq!(b["code"], "project.archived");

    let (s, b) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{existing}/claim"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "claim: {b}");
    assert_eq!(b["code"], "project.archived");

    let (s, b) = app
        .post(
            &app.worker,
            "/v1/questions",
            json!({
                "ticket": existing,
                "kind": "confirm",
                "title": "Should I keep going?",
                "body": "The project froze underneath me.",
            }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "ask: {b}");
    assert_eq!(b["code"], "project.archived");

    let (s, b) = app
        .post(
            &app.admin,
            "/v1/projects/tp/tags",
            json!({ "kind": "person", "handle": "grace" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "tag: {b}");
    assert_eq!(b["code"], "project.archived");

    let (s, b) = app
        .post(
            &app.admin,
            "/v1/projects/tp/environments",
            json!({ "slug": "staging" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "environment: {b}");
    assert_eq!(b["code"], "project.archived");

    // The project's own settings are frozen too: an archived project is kept as
    // it stood, not reconfigured in place.
    let (s, b) = app
        .put(
            &app.admin,
            "/v1/projects/tp/language",
            json!({ "language": "German" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "language: {b}");
    assert_eq!(b["code"], "project.archived");

    // …and every read still answers, unchanged.
    let (s, t) = app
        .get(&app.admin, &format!("/v1/tickets/{existing}"))
        .await;
    assert_eq!(s, StatusCode::OK, "reading a ticket must still work: {t}");
    assert_eq!(t["state"], json!("brief"), "nothing moved: {t}");
    let (s, _) = app.get(&app.admin, "/v1/tickets?project=tp").await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = app.get(&app.admin, "/v1/events?since=0&project=tp").await;
    assert_eq!(s, StatusCode::OK);
    let (s, tags) = app.get(&app.admin, "/v1/projects/tp/tags").await;
    assert_eq!(s, StatusCode::OK, "{tags}");
}

// The undo. Unarchiving restores the project exactly as it stood, because
// archiving never changed it: the write that was refused now succeeds, from the
// state the archive froze the ticket in.
#[tokio::test]
async fn project_unarchive_restores_writes() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Frozen, then thawed").await;

    let (s, _) = app
        .post(&app.admin, "/v1/projects/tp/archive", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = app.transition(&app.human, &id, "spec").await;
    assert_eq!(s, StatusCode::CONFLICT);

    let (s, body) = app
        .post(&app.admin, "/v1/projects/tp/unarchive", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["archived"], json!(false));
    assert_eq!(body["archived_at"], Value::Null);

    let (s, t) = app.transition(&app.human, &id, "spec").await;
    assert_eq!(s, StatusCode::OK, "{t}");
    assert_eq!(t["state"], json!("spec"));
}

// An archived project's tickets leave the ready queue: an agent must never be
// offered work it would then be refused permission to do. The total moves with
// the page, so "n of m" cannot count tickets the queue would not hand over.
#[tokio::test]
async fn project_archive_empties_the_ready_queue() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Ready until the project froze").await;
    app.to_ready(&id).await;

    let (_, ready) = app.get(&app.worker, "/v1/ready?project=tp").await;
    assert_eq!(ready["total"], json!(1), "{ready}");

    let (s, _) = app
        .post(&app.admin, "/v1/projects/tp/archive", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK);

    let (_, ready) = app.get(&app.worker, "/v1/ready?project=tp").await;
    assert_eq!(
        ready["total"],
        json!(0),
        "archived work must not be queued: {ready}"
    );
    assert!(ready["items"].as_array().unwrap().is_empty(), "{ready}");

    // Nothing can be pulled from it either.
    let (s, body) = app.post(&app.worker, "/v1/ready/claim", json!({})).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{body}");

    // Unarchiving puts it back, still ready.
    let (s, _) = app
        .post(&app.admin, "/v1/projects/tp/unarchive", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, ready) = app.get(&app.worker, "/v1/ready?project=tp").await;
    assert_eq!(ready["total"], json!(1), "{ready}");
}

// An active lease refuses the archive with a teaching 409: freezing a project
// under a running worker would leave it no call it could make, not even a
// release. `?force=true` archives anyway and releases the lease, bumping the
// fence so the displaced worker gets a fencing 409 it already knows how to read.
#[tokio::test]
async fn project_archive_refuses_active_claim_unless_forced() {
    let app = TestApp::spawn().await;
    let id = app
        .create_ticket("Claimed while its project is archived")
        .await;
    app.to_ready(&id).await;
    let fence = app.claim_ttl(&app.worker, &id, Some(900)).await;

    let (s, body) = app
        .post(&app.admin, "/v1/projects/tp/archive", json!({}))
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "project.active_claims");
    assert_eq!(body["details"]["active_claims"], json!(1));
    assert_eq!(body["details"]["tickets"][0], json!(id));
    assert!(body["message"].as_str().unwrap().contains("force=true"));

    // The refusal changed nothing: the lease is still the worker's.
    let (_, t) = app.get(&app.worker, &format!("/v1/tickets/{id}")).await;
    assert_eq!(t["claim"]["holder"], json!("agent:w1"), "{t}");

    let (s, body) = app
        .post(&app.admin, "/v1/projects/tp/archive?force=true", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["archived"], json!(true));

    let (_, t) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
    assert_eq!(t["claim"], Value::Null, "the lease must be released: {t}");

    // And the fence moved: even once the project is live again, the displaced
    // worker's echo of its old fence is refused rather than accepted.
    let (s, _) = app
        .post(&app.admin, "/v1/projects/tp/unarchive", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, body) = app
        .patch(
            &app.worker,
            &format!("/v1/tickets/{id}"),
            json!({ "priority": "high", "fence": fence }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "fence.stale");
}

// Archiving is idempotent and keeps the ORIGINAL timestamp: the audit trail says
// when the project was frozen, not when someone last said so. Unarchiving a live
// project is a no-op for the same reason.
#[tokio::test]
async fn project_archive_is_idempotent() {
    let app = TestApp::spawn().await;
    let (s, first) = app
        .post(&app.admin, "/v1/projects/tp/archive", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{first}");
    let (s, second) = app
        .post(&app.admin, "/v1/projects/tp/archive", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{second}");
    assert_eq!(
        first["archived_at"], second["archived_at"],
        "a second archive must not restamp the first"
    );
    let (_, events) = app.get(&app.admin, "/v1/events?since=0&project=tp").await;
    let archived = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "project_archived")
        .count();
    assert_eq!(archived, 1, "one event per actual freeze: {events}");

    let (s, _) = app
        .post(&app.admin, "/v1/projects/tp/unarchive", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, body) = app
        .post(&app.admin, "/v1/projects/tp/unarchive", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["archived"], json!(false));
}

// Archiving is an admin act. An unknown project is a 404, not a silent success.
#[tokio::test]
async fn project_archive_requires_admin() {
    let app = TestApp::spawn().await;
    for token in [&app.human, &app.worker] {
        let (s, body) = app.post(token, "/v1/projects/tp/archive", json!({})).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
        let (s, body) = app
            .post(token, "/v1/projects/tp/unarchive", json!({}))
            .await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    }
    let (s, body) = app
        .post(&app.admin, "/v1/projects/ghost/archive", json!({}))
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["code"], "notfound.project");

    let p = app.project(&app.admin, "tp").await;
    assert_eq!(p["archived"], json!(false), "{p}");
}

// The gate is per project, not global: a second project keeps working while the
// first is frozen.
#[tokio::test]
async fn project_archive_leaves_other_projects_alone() {
    let app = TestApp::spawn().await;
    let (s, _) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({ "id": "other", "name": "Other" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let mine = app.create_ticket("Frozen").await;
    let theirs = app.create_ticket_in("other", "Untouched").await;

    let (s, _) = app
        .post(&app.admin, "/v1/projects/tp/archive", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK);

    let (s, body) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{theirs}/comments"),
            json!({ "body": "still working" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");
    let (s, body) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{mine}/comments"),
            json!({ "body": "not working" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "project.archived");
}

// A ticket may not be moved OUT of an archived project either — emptying a
// frozen project one ticket at a time is exactly what the gate exists to stop,
// and it is the direction a caller is most likely to assume is allowed.
#[tokio::test]
async fn project_archive_blocks_moves_in_both_directions() {
    let app = TestApp::spawn().await;
    let (s, _) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({ "id": "other", "name": "Other" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let from_archived = app.create_ticket("Leaving the frozen project").await;
    let to_archived = app
        .create_ticket_in("other", "Entering the frozen project")
        .await;

    let (s, _) = app
        .post(&app.admin, "/v1/projects/tp/archive", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK);

    let (s, body) = app
        .post(
            &app.admin,
            "/v1/tickets/move",
            json!({ "tickets": [from_archived], "to_project": "other" }),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "out of an archived project: {body}"
    );
    assert_eq!(body["code"], "project.archived");

    let (s, body) = app
        .post(
            &app.admin,
            "/v1/tickets/move",
            json!({ "tickets": [to_archived], "to_project": "tp" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "into an archived project: {body}");
    assert_eq!(body["code"], "project.archived");
}

// --- project delete ---------------------------------------------------------

// DELETE /v1/projects/{id} cascade-removes the project and all of its tickets,
// comments, deps, and events in one shot. 404 for an unknown project.
#[tokio::test]
async fn project_delete_cascades_tickets_and_events() {
    let app = TestApp::spawn().await;
    let a = app.create_ticket("Doomed ticket A").await;
    let b = app.create_ticket("Doomed ticket B blocks A").await;
    // A blocked_by B, and a comment on A — all must vanish with the project.
    let (s, _) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{a}/deps"),
            json!({ "blocked_by": b }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, _) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{a}/comments"),
            json!({ "body": "last words" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);

    // Sanity: events exist for the project before deletion.
    let (_, before) = app.get(&app.admin, "/v1/events?since=0&project=tp").await;
    assert!(!before["events"].as_array().unwrap().is_empty());

    // Unknown project -> 404.
    let (s, body) = app.delete(&app.admin, "/v1/projects/ghost").await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["code"], "notfound.project");

    // Delete succeeds with 204 (no active claims).
    let (s, _body) = app.delete(&app.admin, "/v1/projects/tp").await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // The project is gone.
    let (_, projects) = app.get(&app.admin, "/v1/projects").await;
    let ids: Vec<&str> = projects
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert!(!ids.contains(&"tp"), "project should be gone: {ids:?}");

    // Its tickets are gone (404 on GET).
    let (s, _) = app.get(&app.admin, &format!("/v1/tickets/{a}")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Its per-project events are gone (the audit event is store-level, project=null).
    let (_, after) = app.get(&app.admin, "/v1/events?since=0&project=tp").await;
    assert!(
        after["events"].as_array().unwrap().is_empty(),
        "project events must be cleared: {after}"
    );

    // Deleting again is a 404 (idempotent-ish: it is truly gone).
    let (s, _) = app.delete(&app.admin, "/v1/projects/tp").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

// An active (unexpired) claim blocks delete with a teaching 409; ?force=true
// overrides it.
#[tokio::test]
async fn project_delete_refuses_active_claim_unless_forced() {
    let app = TestApp::spawn().await;
    let id = app
        .create_ticket("Claimed while its project is deleted")
        .await;
    app.to_ready(&id).await;
    // Long lease so it stays active across the test.
    app.claim_ttl(&app.worker, &id, Some(900)).await;

    // Without force: 409 naming the active claim.
    let (s, body) = app.delete(&app.admin, "/v1/projects/tp").await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "project.active_claims");
    assert_eq!(body["details"]["active_claims"], 1);
    assert!(body["message"].as_str().unwrap().contains("force=true"));

    // The project and its ticket still exist (the refusal changed nothing).
    let (s, _) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
    assert_eq!(s, StatusCode::OK);

    // With ?force=true: it deletes anyway.
    let (s, _) = app.delete(&app.admin, "/v1/projects/tp?force=true").await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

// A released (or expired) claim does not block delete: the guard is about
// *active* leases only.
#[tokio::test]
async fn project_delete_allows_after_claim_released() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Released before project delete").await;
    app.to_ready(&id).await;
    let fence = app.claim(&id).await;
    let (s, _) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/release"),
            json!({ "fence": fence }),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // No active claim now: plain delete works without force.
    let (s, _) = app.delete(&app.admin, "/v1/projects/tp").await;
    assert_eq!(s, StatusCode::NO_CONTENT);
}

// Only admin scope may delete a project; read/write is 403'd.
#[tokio::test]
async fn project_delete_requires_admin_scope() {
    let app = TestApp::spawn().await;
    app.create_ticket("Guarded by admin scope").await;

    let (s, body) = app.delete(&app.worker, "/v1/projects/tp").await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "auth.scope");

    // The human scope is not admin either.
    let (s, body) = app.delete(&app.human, "/v1/projects/tp").await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "auth.scope");

    // The project survived the rejected attempts.
    let (_, projects) = app.get(&app.admin, "/v1/projects").await;
    let ids: Vec<&str> = projects
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"tp"), "project must survive a 403: {ids:?}");
}

// Deleting one project leaves other projects — and cross-project dep edges into
// the deleted project — clean.
#[tokio::test]
async fn project_delete_is_scoped_and_clears_cross_project_deps() {
    let app = TestApp::spawn().await;
    // Second project whose ticket is blocked_by a ticket in `tp`.
    let (s, _) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({ "id": "keep", "name": "Keeper" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, t) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "keep", "title": "Survivor ticket" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let keeper = t["id"].as_str().unwrap().to_string();
    let blocker = app.create_ticket("tp ticket blocking a keep ticket").await;
    // keep-ticket blocked_by a tp-ticket (blocked_by is not project-scoped).
    let (s, _) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{keeper}/deps"),
            json!({ "blocked_by": blocker }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);

    // Delete tp; keep must be untouched and the dangling dep edge cleared.
    let (s, _) = app.delete(&app.admin, "/v1/projects/tp").await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    let (_, projects) = app.get(&app.admin, "/v1/projects").await;
    let ids: Vec<&str> = projects
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"keep") && !ids.contains(&"tp"), "{ids:?}");

    // The keep ticket survives with its dangling blocker edge removed.
    let (s, kt) = app.get(&app.admin, &format!("/v1/tickets/{keeper}")).await;
    assert_eq!(s, StatusCode::OK, "{kt}");
    assert!(
        kt["blocked_by"].as_array().unwrap().is_empty(),
        "cross-project dep into a deleted project must be cleared: {kt}"
    );
}

// Regression: deleting a project that has ever carried a question, a tag, an
// answer link or a promotion used to 500. Each of those tables holds a real
// REFERENCES into questions/tickets/projects, so a cascade that skips them hits
// the immediate foreign-key check and aborts the whole transaction. The other
// delete tests never create any of them, which is how this survived.
#[tokio::test]
async fn project_delete_cascades_questions_tags_grants_and_promotions() {
    let app = TestApp::spawn().await;

    // A tagged ticket — creating with `tags` auto-registers the `person:ada`
    // stub in the project tag registry (tags REFERENCES projects(id)) — plus a
    // registry entry no ticket refers to.
    let (s, tagged) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "tp", "title": "Tagged before its project is deleted", "tags": ["person:ada"] }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{tagged}");
    let tagged = tagged["id"].as_str().unwrap().to_string();
    let (s, reg) = app
        .post(
            &app.admin,
            "/v1/projects/tp/tags",
            json!({ "kind": "component", "handle": "billing" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{reg}");

    // A promotion (REFERENCES tickets(id)).
    let (s, promo) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{tagged}/promote"),
            json!({ "target": "staging" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{promo}");

    // A blocking question (REFERENCES projects(id) AND tickets(id)) with a
    // follow-up thread (question_messages) and a minted answer link
    // (answer_grants), both of which REFERENCE questions(id).
    let asked = app.create_ticket("Parked on a human decision").await;
    let fence = app.to_implementing(&asked).await;
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": asked, "kind": "confirm", "title": "OK to drop the table?", "fence": fence }),
        )
        .await;
    let (s, fu) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/followup"),
            json!({ "message": "How many rows and how long is the lock?" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{fu}");
    let (s, rep) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{qid}/reply"),
            json!({ "message": "40k rows, ~2s lock, reversible." }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{rep}");
    let (s, link) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/answer-link"),
            json!({ "actor": "human:contractor" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{link}");

    // Asking parked the ticket and released the lease, so no force is needed:
    // this must be a clean 204, not a 500 from an aborted transaction.
    let (s, body) = app.delete(&app.admin, "/v1/projects/tp").await;
    assert_eq!(
        s,
        StatusCode::NO_CONTENT,
        "delete must cascade the question/tag/grant/promotion tables: {body}"
    );

    // Nothing survives in the tables the cascade used to skip. Read them
    // straight from SQLite: some have no list endpoint once the project is gone.
    let conn = rusqlite::Connection::open(app.db_path()).expect("open db");
    for table in [
        "questions",
        "question_messages",
        "answer_grants",
        "tags",
        "promotions",
    ] {
        let n: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .expect("count");
        assert_eq!(n, 0, "{table} rows survived the project delete");
    }

    // The audit event accounts for every table it cleared.
    let payload: String = conn
        .query_row(
            "SELECT payload FROM events WHERE kind = 'project_deleted'",
            [],
            |r| r.get(0),
        )
        .expect("audit event");
    let payload: Value = serde_json::from_str(&payload).expect("payload json");
    let deleted = &payload["deleted"];
    assert_eq!(deleted["tickets"], 2, "{deleted}");
    assert_eq!(deleted["questions"], 1, "{deleted}");
    assert_eq!(deleted["question_messages"], 2, "{deleted}");
    assert_eq!(deleted["answer_grants"], 1, "{deleted}");
    assert_eq!(deleted["tags"], 2, "{deleted}");
    assert_eq!(deleted["promotions"], 1, "{deleted}");

    // And the project itself is gone.
    let (s, _) = app.delete(&app.admin, "/v1/projects/tp").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn roadmap_rolls_up_epic_subtree() {
    let app = TestApp::spawn().await;

    // epic
    //  |- child_a
    //  |    \- grandchild   (done)
    //  |- child_a           (done, after its only child is done)
    //  |- child_b           (brief)
    //  |- child_c           (brief)
    //  \- child_d           (brief)
    let epic = app.create_typed("Ship the widget", "epic", None).await;
    let child_a = app.create_typed("Backend", "task", Some(&epic)).await;
    let grandchild = app
        .create_typed("Backend subtask", "task", Some(&child_a))
        .await;
    let _child_b = app.create_typed("Frontend", "task", Some(&epic)).await;
    let _child_c = app.create_typed("Docs", "task", Some(&epic)).await;
    let _child_d = app.create_typed("QA", "task", Some(&epic)).await;

    // Finish the grandchild first, then child_a (its subtree is now clear).
    app.drive_to_done(&grandchild).await;
    app.drive_to_done(&child_a).await;

    let (status, body) = app.get(&app.admin, "/v1/projects/tp/roadmap").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["project"], "tp");
    let epics = body["epics"].as_array().expect("epics array");
    assert_eq!(epics.len(), 1, "one epic expected: {body}");
    let e = &epics[0];
    assert_eq!(e["id"], epic.as_str());

    // Full descendant subtree = child_a + grandchild + child_b + child_c + child_d = 5.
    assert_eq!(e["total"], 5, "subtree total: {e}");
    // Two are done (child_a + grandchild); percent = round(2/5*100) = 40.
    assert_eq!(e["done"], 2, "done count: {e}");
    assert_eq!(e["percent"], 40, "percent: {e}");
    assert_eq!(e["by_category"]["done"], 2, "by_category done: {e}");
    assert_eq!(e["by_category"]["todo"], 3, "by_category todo: {e}");
    assert_eq!(e["by_state"]["done"], 2, "by_state done: {e}");
    assert_eq!(e["by_state"]["brief"], 3, "by_state brief: {e}");

    // The epic itself is the container, not counted in its own rollup.
    assert!(
        e["by_state"].get("spec").is_none(),
        "epic not self-counted: {e}"
    );

    // Unknown project -> 404.
    let (status, body) = app.get(&app.admin, "/v1/projects/nope/roadmap").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

// The `unparented` bucket catches every way a ticket can end up outside all
// epics: no parent at all, a chain of non-epic ancestors, and a parent id that
// points at a row that is not there. Counts must stay coherent — with flat
// epics, every non-epic ticket lands in exactly one bucket.
#[tokio::test]
async fn roadmap_unparented_bucket_covers_every_orphan_shape() {
    let app = TestApp::spawn().await;

    // Two flat epics, so no ticket is counted by two epic subtrees.
    let epic_a = app.create_typed("Owned work", "epic", None).await;
    let owned_done = app.create_typed("Owned A", "task", Some(&epic_a)).await;
    let _owned_open = app.create_typed("Owned B", "task", Some(&epic_a)).await;
    app.drive_to_done(&owned_done).await;
    let _epic_b = app.create_typed("Planned work", "epic", None).await;

    // 1. No parent at all.
    let loose = app.create_typed("Loose task", "task", None).await;
    app.drive_to_done(&loose).await;
    // 2. A chain of non-epic ancestors: leaf -> mid -> (nothing).
    let mid = app.create_typed("Mid task", "task", None).await;
    let _leaf = app.create_typed("Leaf task", "task", Some(&mid)).await;
    // 3. A dangling parent: the row it points at does not exist.
    let dangling = app.create_typed("Dangling task", "task", None).await;
    app.force_parent(&dangling, "tp-nosuchrow");

    let (status, body) = app.get(&app.admin, "/v1/projects/tp/roadmap").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let u = &body["unparented"];

    // loose + mid + leaf + dangling = 4; the two epics themselves never count.
    assert_eq!(u["total"], 4, "unparented total: {body}");
    assert_eq!(u["done"], 1, "only the loose task is done: {body}");
    assert_eq!(u["percent"], 25, "round(1/4*100): {body}");
    assert_eq!(u["by_state"]["done"], 1, "{body}");
    assert_eq!(u["by_state"]["brief"], 3, "{body}");
    assert_eq!(u["by_category"]["done"], 1, "{body}");
    assert_eq!(u["by_category"]["todo"], 3, "{body}");
    assert!(
        u.get("id").is_none() && u.get("title").is_none() && u.get("state").is_none(),
        "the bucket is not a ticket: {u}"
    );

    // Coherence: epics are flat here, so every non-epic ticket is counted
    // exactly once across the epic subtrees plus the unparented bucket.
    let epic_total: i64 = body["epics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["total"].as_i64().unwrap())
        .sum();
    let (_, list) = app
        .get(&app.admin, "/v1/tickets?project=tp&limit=200")
        .await;
    let non_epics = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["type"] != "epic")
        .count() as i64;
    assert_eq!(
        epic_total + u["total"].as_i64().unwrap(),
        non_epics,
        "epic subtotals + unparented must account for every non-epic ticket: {body}"
    );
}

// A `parent` cycle is not reachable through the API, but a corrupted database
// can hold one. Both recursive walks use UNION, which stops at an already-seen
// id — the endpoint must answer rather than spin.
#[tokio::test]
async fn roadmap_terminates_on_parent_cycles() {
    let app = TestApp::spawn().await;

    // A cycle through an epic: epic -> p -> epic. The subtree walk revisits the
    // epic and must stop there.
    let epic = app.create_typed("Cyclic epic", "epic", None).await;
    let p = app.create_typed("Under epic", "task", Some(&epic)).await;
    app.force_parent(&epic, &p);

    // A cycle with no epic anywhere above it: x <-> y, plus a tail hanging off
    // it whose upward chain runs into the cycle forever.
    let x = app.create_typed("Free one", "task", None).await;
    let y = app.create_typed("Free two", "task", Some(&x)).await;
    app.force_parent(&x, &y);
    let _tail = app
        .create_typed("Tail of the cycle", "task", Some(&x))
        .await;

    let (status, body) = tokio::time::timeout(
        Duration::from_secs(10),
        app.get(&app.admin, "/v1/projects/tp/roadmap"),
    )
    .await
    .expect("roadmap must terminate on a parent cycle, not hang");
    assert_eq!(status, StatusCode::OK, "{body}");

    // The walk reaches p and, around the cycle, the epic itself — each once.
    let e = &body["epics"].as_array().unwrap()[0];
    assert_eq!(e["total"], 2, "cyclic subtree counted once each: {body}");
    // x, y and the tail never reach an epic upward, so all three are unparented
    // (the epic in the cycle is an epic, and epics never join the bucket).
    assert_eq!(body["unparented"]["total"], 3, "{body}");
}

// Each flag is a pure derivation over the epic's own state category and its
// subtree counts.
#[tokio::test]
async fn roadmap_flags_epic_state_contradictions() {
    let app = TestApp::spawn().await;

    // 1. done epic, open children: the child is cancelled (terminal, so the
    //    done guard passes) but not done, leaving done(0) < total(1).
    let e_done_open = app.create_typed("Shipped early", "epic", None).await;
    let stray = app
        .create_typed("Cancelled child", "task", Some(&e_done_open))
        .await;
    let (s, b) = app.transition(&app.human, &stray, "cancelled").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    app.drive_to_done(&e_done_open).await;

    // 2. open epic, all children done.
    let e_open_all_done = app.create_typed("Work finished", "epic", None).await;
    let child = app
        .create_typed("Only child", "task", Some(&e_open_all_done))
        .await;
    app.drive_to_done(&child).await;

    // 3. an epic with no descendants at all.
    let e_empty = app.create_typed("Filed ahead", "epic", None).await;

    // 4. empty *and* done: `empty_epic` fires, `done_with_open_children` must
    //    not — done < total is false when total is 0.
    let e_empty_done = app.create_typed("Empty and done", "epic", None).await;
    app.drive_to_done(&e_empty_done).await;

    // 5. a consistent epic: in progress with a mix of open children.
    let e_ok = app.create_typed("Business as usual", "epic", None).await;
    let ok_done = app.create_typed("Done bit", "task", Some(&e_ok)).await;
    let _ok_open = app.create_typed("Open bit", "task", Some(&e_ok)).await;
    app.drive_to_done(&ok_done).await;

    let (status, body) = app.get(&app.admin, "/v1/projects/tp/roadmap").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let flags = |id: &str| -> Vec<String> {
        body["epics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == id)
            .unwrap_or_else(|| panic!("epic {id} missing: {body}"))["flags"]
            .as_array()
            .unwrap_or_else(|| panic!("epic {id} has no flags array: {body}"))
            .iter()
            .map(|f| f.as_str().unwrap().to_string())
            .collect()
    };

    assert_eq!(flags(&e_done_open), ["done_with_open_children"], "{body}");
    assert_eq!(
        flags(&e_open_all_done),
        ["open_with_all_children_done"],
        "{body}"
    );
    assert_eq!(flags(&e_empty), ["empty_epic"], "{body}");
    assert_eq!(
        flags(&e_empty_done),
        ["empty_epic"],
        "an empty done epic is empty only — done < total cannot hold at total 0: {body}"
    );
    assert!(
        flags(&e_ok).is_empty(),
        "a consistent epic carries no flags: {body}"
    );
}

// The rollup splits claimable work into `ready` and `backlog`, and `ready` must
// mean exactly what the ready QUEUE means — a rollup that disagreed with the
// thing that actually hands work out would be worse than no rollup.
//
// The split keys on the workflow's `claimable` FLAG, not the `todo` category,
// and this workflow is why that matters: `brief` is category `todo` and NOT
// claimable, while `spec` is claimable and categorised `in_progress`. A
// category-based split gets both wrong, in opposite directions.
#[tokio::test]
async fn roadmap_splits_claimable_work_into_ready_and_backlog() {
    let app = TestApp::spawn().await;

    let epic = app.create_typed("Ship the widget", "epic", None).await;
    // brief: category todo, NOT claimable -> neither ready nor backlog.
    let _briefed = app.create_typed("Still a brief", "task", Some(&epic)).await;
    // ready + unblocked + unclaimed -> ready.
    let free = app
        .create_typed("Free to pick up", "task", Some(&epic))
        .await;
    app.to_ready(&free).await;
    // ready but blocked by an open ticket -> backlog.
    let blocked = app.create_typed("Held by a dep", "task", Some(&epic)).await;
    app.to_ready(&blocked).await;
    // The blocker itself is claimable and unblocked, so it is READY — closing it
    // is precisely the work that releases the other one.
    let blocker = app.create_typed("The blocker", "task", Some(&epic)).await;
    app.to_ready(&blocker).await;
    let (s, b) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{blocked}/deps"),
            json!({ "blocked_by": blocker }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "dep add failed: {b}");
    // ready but already claimed -> backlog (someone holds the lease).
    let taken = app
        .create_typed("Already claimed", "task", Some(&epic))
        .await;
    app.to_ready(&taken).await;
    app.claim(&taken).await;

    let (status, body) = app.get(&app.admin, "/v1/projects/tp/roadmap").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let e = &body["epics"][0];

    // `free` and `blocker` are the two the queue would hand out — `blocker` is
    // itself unblocked, which is the whole point of it being the blocker.
    assert_eq!(e["ready"], 2, "ready: {e}");
    // `blocked` (dep) + `taken` (claimed). `briefed` is NOT here: it sits in a
    // non-claimable state, so it is neither ready nor backlog.
    assert_eq!(e["backlog"], 2, "backlog: {e}");

    // The assertion this whole split exists for: `ready` must equal what the
    // ready QUEUE actually hands out. Both read the same CTE by construction
    // (BLOCKED_CTE is shared), and this pins that they cannot drift apart.
    let (s, queue) = app.get(&app.admin, "/v1/ready?project=tp").await;
    assert_eq!(s, StatusCode::OK, "{queue}");
    let queue_len = queue["items"].as_array().expect("ready items").len() as i64;
    assert_eq!(
        e["ready"].as_i64().expect("ready count"),
        queue_len,
        "rollup `ready` must equal /v1/ready: {e} vs {queue}"
    );
}

// `awaiting_answer` is an OVERLAY on the state counts, not a bucket: a ticket
// with an open question is still counted in its own state. It exists because a
// queue entry carrying an unanswered decision looks claimable and is not worth
// picking up.
#[tokio::test]
async fn roadmap_counts_tickets_awaiting_an_answer() {
    let app = TestApp::spawn().await;

    let epic = app.create_typed("Needs decisions", "epic", None).await;
    let asked = app
        .create_typed("Has a question", "task", Some(&epic))
        .await;
    let _quiet = app.create_typed("No question", "task", Some(&epic)).await;

    let fence = app.to_implementing(&asked).await;
    let (s, b) = app
        .post(
            &app.worker,
            "/v1/questions",
            json!({
                "ticket": asked,
                "kind": "choose",
                "title": "Which way?",
                "options": ["left", "right"],
                "fence": fence,
            }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "ask failed: {b}");

    let (status, body) = app.get(&app.admin, "/v1/projects/tp/roadmap").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let e = &body["epics"][0];
    assert_eq!(e["awaiting_answer"], 1, "awaiting_answer: {e}");
    // Overlay, not a partition: the ticket is still counted in its own state, so
    // the subtree total is untouched by the question.
    assert_eq!(e["total"], 2, "total unchanged by the question: {e}");
}

/// Open an initiative over HTTP and return its id.
async fn new_initiative(app: &TestApp, title: &str, status: &str) -> String {
    let (s, b) = app
        .post(
            &app.admin,
            "/v1/initiatives",
            json!({ "project": "tp", "title": title, "status": status }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "initiative create failed: {b}");
    b["id"].as_str().expect("initiative id").to_string()
}

/// Create a ticket carrying one `initiative:<id>` tag.
async fn create_tagged(app: &TestApp, title: &str, ty: &str, tag: &str) -> String {
    let (s, b) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "tp", "title": title, "type": ty, "tags": [tag] }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "tagged create failed: {b}");
    b["id"].as_str().expect("ticket id").to_string()
}

// An epic closes; an initiative does not. So a feature shipping as v1 then v2 is
// ONE initiative with one epic per version, and the roadmap has to report both
// readings at once: per-version progress from `epics`, lane-lifetime progress
// from `initiatives`. This is the whole point of the initiative rollup.
#[tokio::test]
async fn roadmap_rolls_up_an_initiative_across_its_version_epics() {
    let app = TestApp::spawn().await;
    let ini = new_initiative(&app, "Billing", "open").await;
    let tag = format!("initiative:{ini}");

    // v1: two tasks, both done — the version that shipped.
    let v1 = create_tagged(&app, "Billing v1", "epic", &tag).await;
    let v1_a = app.create_typed("Charge card", "task", Some(&v1)).await;
    let v1_b = app.create_typed("Send receipt", "task", Some(&v1)).await;
    app.drive_to_done(&v1_a).await;
    app.drive_to_done(&v1_b).await;

    // v2: two tasks, one done. A grandchild proves the walk goes all the way
    // down from the tagged epic rather than stopping at its children.
    let v2 = create_tagged(&app, "Billing v2", "epic", &tag).await;
    let v2_a = app.create_typed("Proration", "task", Some(&v2)).await;
    let v2_deep = app.create_typed("Rounding", "task", Some(&v2_a)).await;
    let _v2_b = app.create_typed("Invoices", "task", Some(&v2)).await;
    app.drive_to_done(&v2_deep).await;

    let (status, body) = app.get(&app.admin, "/v1/projects/tp/roadmap").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let lanes = body["initiatives"].as_array().expect("initiatives array");
    assert_eq!(lanes.len(), 1, "one initiative expected: {body}");
    let lane = &lanes[0];
    assert_eq!(lane["id"], ini.as_str());
    assert_eq!(lane["title"], "Billing");
    assert_eq!(lane["status"], "open");

    // The lane spans both versions: 2 (v1) + 3 (v2 subtree) = 5 tickets, 3 done.
    // The epics themselves are containers and are not counted.
    assert_eq!(lane["total"], 5, "lane spans both version epics: {lane}");
    assert_eq!(lane["done"], 3, "lane done count: {lane}");
    assert_eq!(lane["percent"], 60, "round(3/5*100): {lane}");
    assert_eq!(lane["by_category"]["done"], 3, "{lane}");
    assert_eq!(lane["by_category"]["todo"], 2, "{lane}");
    assert!(
        lane["flags"].as_array().unwrap().is_empty(),
        "a fed, open lane is consistent: {lane}"
    );

    // The versions are named, in creation order, so a client can join them to
    // the per-epic rollups in the same response.
    assert_eq!(
        lane["epics"],
        json!([v1.as_str(), v2.as_str()]),
        "versions filed under the lane: {lane}"
    );

    // And the per-version reading is untouched: v1 is complete, v2 is not.
    let epics = body["epics"].as_array().expect("epics array");
    let e1 = epics.iter().find(|e| e["id"] == v1.as_str()).expect("v1");
    let e2 = epics.iter().find(|e| e["id"] == v2.as_str()).expect("v2");
    assert_eq!(e1["percent"], 100, "v1 shipped: {e1}");
    assert_eq!(e2["total"], 3, "v2 subtree: {e2}");
    assert_eq!(e2["done"], 1, "v2 partly done: {e2}");

    // Everything is inside the lane, so nothing is left over.
    assert_eq!(body["uninitiated"]["total"], 0, "{body}");
}

// The initiative-side twin of the `unparented` bucket. It has to catch the same
// shapes, including the one that is easy to lose: a tag naming an initiative
// that does not exist. Such a ticket belongs to no lane, so it must land here
// rather than vanish from every bucket.
#[tokio::test]
async fn roadmap_uninitiated_bucket_covers_work_no_lane_owns() {
    let app = TestApp::spawn().await;
    let ini = new_initiative(&app, "Owned lane", "open").await;
    let tag = format!("initiative:{ini}");

    // Owned: a tagged task plus a child that inherits the lane.
    let owned = create_tagged(&app, "Owned task", "task", &tag).await;
    let _owned_child = app.create_typed("Owned child", "task", Some(&owned)).await;

    // 1. No initiative tag at all, and not under anything that has one.
    let loose = app.create_typed("Loose task", "task", None).await;
    app.drive_to_done(&loose).await;
    // 2. Under an epic, but the epic carries no initiative tag either.
    let orphan_epic = app.create_typed("Unfiled epic", "epic", None).await;
    let _under_epic = app
        .create_typed("Task under unfiled epic", "task", Some(&orphan_epic))
        .await;
    // 3. A tag pointing at an initiative that does not exist.
    let _dangling = create_tagged(&app, "Stale lane ref", "task", "initiative:ini-nosuchid").await;

    let (status, body) = app.get(&app.admin, "/v1/projects/tp/roadmap").await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let lane = &body["initiatives"][0];
    assert_eq!(lane["total"], 2, "tagged task + its child: {lane}");

    let u = &body["uninitiated"];
    // loose + under_epic + dangling = 3. The epics themselves never count.
    assert_eq!(u["total"], 3, "uninitiated total: {body}");
    assert_eq!(u["done"], 1, "only the loose task is done: {body}");
    assert_eq!(u["percent"], 33, "round(1/3*100): {body}");
    assert!(
        u.get("id").is_none() && u.get("title").is_none() && u.get("status").is_none(),
        "the bucket is not an initiative: {u}"
    );

    // Coherence: with one flat lane, every non-epic ticket is counted exactly
    // once across the lane and the uninitiated bucket.
    let (_, list) = app
        .get(&app.admin, "/v1/tickets?project=tp&limit=200")
        .await;
    let non_epics = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["type"] != "epic")
        .count() as i64;
    assert_eq!(
        lane["total"].as_i64().unwrap() + u["total"].as_i64().unwrap(),
        non_epics,
        "every non-epic ticket counted exactly once: {body}"
    );
}

// An initiative's status is a LABEL, not a state machine, so its flags are not
// the epic's. `distilled` with open work is correct, not a contradiction —
// distilled means the substance became tickets. What is worth surfacing is a
// lane deliberately set aside whose tickets the queue is still handing out.
#[tokio::test]
async fn roadmap_flags_initiative_status_contradictions() {
    let app = TestApp::spawn().await;

    // Filed ahead of its work: a flag, not an error.
    let _empty = new_initiative(&app, "Nothing filed yet", "open").await;

    // Parked, but a ticket under it is sitting in the ready queue.
    let parked = new_initiative(&app, "Set aside", "parked").await;
    let parked_work = create_tagged(
        &app,
        "Still claimable",
        "task",
        &format!("initiative:{parked}"),
    )
    .await;
    app.to_ready(&parked_work).await;

    // Distilled with open work — the expected shape, and not flagged.
    let distilled = new_initiative(&app, "Became tickets", "distilled").await;
    let _distilled_work = create_tagged(
        &app,
        "Filed from a passage",
        "task",
        &format!("initiative:{distilled}"),
    )
    .await;

    let (status, body) = app.get(&app.admin, "/v1/projects/tp/roadmap").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let lanes = body["initiatives"].as_array().expect("initiatives");
    let by_title = |t: &str| {
        lanes
            .iter()
            .find(|l| l["title"] == t)
            .unwrap_or_else(|| panic!("lane {t} missing: {body}"))
            .clone()
    };

    assert_eq!(
        by_title("Nothing filed yet")["flags"],
        json!(["empty_initiative"]),
        "{body}"
    );

    let p = by_title("Set aside");
    assert_eq!(p["ready"], 1, "the parked lane has claimable work: {p}");
    assert_eq!(p["flags"], json!(["parked_with_ready_work"]), "{p}");

    let d = by_title("Became tickets");
    assert_eq!(d["total"], 1, "{d}");
    assert!(
        d["flags"].as_array().unwrap().is_empty(),
        "distilled with open work is the expected shape, not a contradiction: {d}"
    );
}

// The rollups' `ready` must answer the same question the queue answers, and the
// way that breaks is silent: a rule is added to the queue and not to the rollup,
// and the roadmap goes on advertising work nobody can be handed. That is exactly
// what happened when epic reservations landed — the rollup carried its own copy
// of the predicate — so this pins the two together against the newest rule.
#[tokio::test]
async fn roadmap_ready_agrees_with_the_queue_under_an_epic_reservation() {
    let app = TestApp::spawn().await;
    let ini = new_initiative(&app, "Payments", "open").await;
    let epic = create_tagged(&app, "Payments v1", "epic", &format!("initiative:{ini}")).await;
    let c1 = app.create_typed("Charges", "task", Some(&epic)).await;
    let c2 = app.create_typed("Refunds", "task", Some(&epic)).await;
    app.to_ready(&epic).await;
    app.to_ready(&c1).await;
    app.to_ready(&c2).await;

    // `type=task` keeps the epic's own readiness out of the comparison: the
    // roadmap treats an epic as a container and never counts it in its own
    // rollup, so the two would differ by one for a reason that is not drift.
    let queue = "/v1/ready?project=tp&type=task&limit=100";
    let (s, q) = app.get(&app.worker, queue).await;
    assert_eq!(s, StatusCode::OK, "{q}");
    assert_eq!(q["total"], 2, "the queue offers both children: {q}");

    let (_, body) = app.get(&app.admin, "/v1/projects/tp/roadmap").await;
    assert_eq!(body["epics"][0]["ready"], 2, "epic rollup agrees: {body}");
    assert_eq!(
        body["initiatives"][0]["ready"], 2,
        "lane rollup agrees: {body}"
    );

    // Claiming the epic reserves its whole subtree, so the queue stops offering
    // the children to anyone.
    let (s, lease) = app
        .post(&app.worker, &format!("/v1/tickets/{epic}/claim"), json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{lease}");

    let (_, q) = app.get(&app.worker, queue).await;
    assert_eq!(q["total"], 0, "a reserved subtree is not offered: {q}");

    // …and every rollup has to say so too. These are the assertions that fail
    // when the rollup keeps its own copy of the readiness predicate.
    let (_, body) = app.get(&app.admin, "/v1/projects/tp/roadmap").await;
    assert_eq!(
        body["epics"][0]["ready"], 0,
        "epic rollup must follow the queue into the reservation: {body}"
    );
    assert_eq!(
        body["initiatives"][0]["ready"], 0,
        "lane rollup must follow it too: {body}"
    );

    // The work did not vanish, it stopped being offerable: `ready + backlog` is
    // the whole claimable set, so it moved across that line.
    assert_eq!(
        body["epics"][0]["backlog"], 2,
        "reserved work is backlog, not gone: {body}"
    );
}

// The other half of the shared predicate. Epic reservations live in the recursive
// CTEs; this rule lives in the outer conditions, so the two tests together pin
// both halves of what the rollup borrows from the queue.
#[tokio::test]
async fn roadmap_ready_follows_the_queue_when_the_project_is_archived() {
    let app = TestApp::spawn().await;
    let epic = app.create_typed("Shipping", "epic", None).await;
    let child = app.create_typed("Rates", "task", Some(&epic)).await;
    app.to_ready(&child).await;

    let queue = "/v1/ready?project=tp&type=task&limit=100";
    let (_, q) = app.get(&app.worker, queue).await;
    assert_eq!(q["total"], 1, "offered before the archive: {q}");
    let (_, body) = app.get(&app.admin, "/v1/projects/tp/roadmap").await;
    assert_eq!(body["epics"][0]["ready"], 1, "{body}");

    let (s, b) = app
        .post(&app.admin, "/v1/projects/tp/archive", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");

    let (_, q) = app.get(&app.worker, queue).await;
    assert_eq!(q["total"], 0, "an archived project offers nothing: {q}");

    // Archiving is a gate on WRITES, not a state, so every read still works —
    // which is exactly why the rollup has to apply the rule itself. A count that
    // still said 1 would invite someone to pick up work the gate then refuses.
    let (s, body) = app.get(&app.admin, "/v1/projects/tp/roadmap").await;
    assert_eq!(
        s,
        StatusCode::OK,
        "the rollup is a read and keeps working: {body}"
    );
    assert_eq!(
        body["epics"][0]["ready"], 0,
        "rollup must follow the queue: {body}"
    );
    assert_eq!(
        body["epics"][0]["backlog"], 1,
        "still in a claimable state, just not offerable: {body}"
    );
}

// An epic claim has no expiry to judge it by, so the roadmap reports who holds it
// and whether anything under them has moved — the numbers an epic-first view needs
// without one event-log walk per epic. And each epic names the lanes it belongs
// to, the inverse of a lane's `epics`, so "what is this version part of" does not
// require inverting a whole project's lanes client-side.
#[tokio::test]
async fn roadmap_epics_carry_their_claim_and_the_lanes_they_belong_to() {
    let app = TestApp::spawn().await;
    let ini = new_initiative(&app, "Payments", "open").await;
    let other = new_initiative(&app, "Reporting", "open").await;
    // Filed under two lanes: legitimate, and the reason the field is a list.
    let epic = create_tagged(&app, "Payments v1", "epic", &format!("initiative:{ini}")).await;
    let (s, b) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{epic}"),
            json!({ "tags_add": [format!("initiative:{other}")] }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let child = app.create_typed("Charges", "task", Some(&epic)).await;
    let unfiled = app.create_typed("Unfiled epic", "epic", None).await;

    // Unclaimed: `claim` is null rather than an object of nulls, so "held" is one
    // check for a client rather than three.
    let (_, body) = app.get(&app.admin, "/v1/projects/tp/roadmap").await;
    let mine = body["epics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == epic.as_str())
        .expect("epic")
        .clone();
    assert!(mine["claim"].is_null(), "unclaimed reads as null: {mine}");
    // Both lanes, in a stable order.
    let mut lanes: Vec<&str> = mine["initiatives"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    lanes.sort();
    let mut want = vec![ini.as_str(), other.as_str()];
    want.sort();
    assert_eq!(lanes, want, "both lanes named: {mine}");

    let unfiled_row = body["epics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == unfiled.as_str())
        .expect("unfiled epic")
        .clone();
    assert_eq!(
        unfiled_row["initiatives"],
        json!([]),
        "an epic under no lane reports an empty list, not null: {unfiled_row}"
    );

    // Claimed with no TTL: held until released, which is why movement is what
    // there is to judge it by.
    app.to_ready(&epic).await;
    app.to_ready(&child).await;
    let (s, lease) = app
        .post(&app.worker, &format!("/v1/tickets/{epic}/claim"), json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{lease}");

    let (_, body) = app.get(&app.admin, "/v1/projects/tp/roadmap").await;
    let mine = body["epics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == epic.as_str())
        .expect("epic")
        .clone();
    let c = &mine["claim"];
    assert_eq!(c["holder"], "agent:w1", "{c}");
    assert_eq!(c["indefinite"], true, "no TTL means no expiry: {c}");
    assert!(c["expires_at"].is_null(), "{c}");
    assert!(c["held_since"].is_string(), "{c}");
    assert!(c["held_for_seconds"].as_i64().unwrap() >= 0, "{c}");
    // Anchored at the claim when nothing has moved since, so a fresh hold reads
    // as barely idle rather than as idle since whenever the subtree last changed.
    assert!(
        c["idle_seconds"].as_i64().unwrap() < 60,
        "a fresh claim is not reported as long-idle: {c}"
    );

    // An expired lease is not a hold: a bounded claim, backdated past its
    // deadline, must not read as claimed.
    app.to_ready(&unfiled).await;
    let (s, b) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{unfiled}/claim"),
            json!({ "ttl_seconds": 1 }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    app.force_claim_expiry(&unfiled, 1_000);
    let (_, body) = app.get(&app.admin, "/v1/projects/tp/roadmap").await;
    let expired = body["epics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == unfiled.as_str())
        .expect("unfiled epic")
        .clone();
    assert!(
        expired["claim"].is_null(),
        "an expired lease is not a hold: {expired}"
    );
}

// Impact ranks blockers by how much closing ONE of them would release. The
// count is a counterfactual, and the cases below are exactly the ones a naive
// reachability walk gets wrong.
#[tokio::test]
async fn impact_ranks_blockers_by_what_closing_one_would_release() {
    let app = TestApp::spawn().await;

    // root blocks `held`, and `held` has a child that inherits the block.
    let root = app.create_typed("Root blocker", "task", None).await;
    let held = app.create_typed("Held directly", "task", None).await;
    let inherited = app.create_typed("Child of held", "task", Some(&held)).await;
    // `shared` is held by TWO blockers, so neither one alone releases it.
    let other = app.create_typed("Second blocker", "task", None).await;
    let shared = app.create_typed("Held by two", "task", None).await;

    for (ticket, blocker) in [(&held, &root), (&shared, &root), (&shared, &other)] {
        let (s, b) = app
            .post(
                &app.admin,
                &format!("/v1/tickets/{ticket}/deps"),
                json!({ "blocked_by": blocker }),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "dep add failed: {b}");
    }

    let (status, body) = app.get(&app.admin, "/v1/projects/tp/impact").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // held + inherited + shared are blocked right now.
    assert_eq!(body["blocked_total"], 3, "{body}");

    let blockers = body["blockers"].as_array().expect("blockers array");
    let top = &blockers[0];
    assert_eq!(top["id"], root.as_str(), "root ranks first: {body}");
    // Closing `root` releases `held` (direct) and `inherited` (downstream) — but
    // NOT `shared`, which `other` still holds. That exclusion is the whole
    // reason this is a counterfactual rather than a graph walk.
    assert_eq!(top["unblocks"], 2, "unblocks: {top}");
    assert_eq!(top["direct"], 1, "direct: {top}");
    assert_eq!(top["downstream"], 1, "downstream: {top}");
    let ids = top["unblocks_ids"].as_array().expect("unblocks_ids");
    assert!(
        ids.iter().any(|v| v == &json!(held))
            && ids.iter().any(|v| v == &json!(inherited))
            && !ids.iter().any(|v| v == &json!(shared)),
        "released set names held + inherited and excludes shared: {top}"
    );

    // `other` alone releases nothing — its only dependent is also held by root.
    // A blocker that frees nothing is omitted rather than listed as a zero.
    assert!(
        !blockers.iter().any(|b| b["id"] == other.as_str()),
        "a blocker that releases nothing is not listed: {body}"
    );

    // Closing the root for real drops it out of the ranking.
    app.drive_to_done(&root).await;
    let (_, body) = app.get(&app.admin, "/v1/projects/tp/impact").await;
    assert!(
        !body["blockers"]
            .as_array()
            .expect("blockers array")
            .iter()
            .any(|b| b["id"] == root.as_str()),
        "a terminal ticket is no longer a blocker: {body}"
    );

    // Unknown project -> 404, like roadmap: an empty list would hide a typo.
    let (status, body) = app.get(&app.admin, "/v1/projects/nope/impact").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

#[tokio::test]
async fn deps_reverse_and_transitive_are_cycle_safe() {
    let app = TestApp::spawn().await;
    let a = app.create_ticket("A depends on B").await;
    let b = app.create_ticket("B depends on C").await;
    let c = app.create_ticket("C the root blocker").await;

    // A blocked_by B, B blocked_by C — a two-hop chain.
    for (t, dep) in [(&a, &b), (&b, &c)] {
        let (s, body) = app
            .post(
                &app.admin,
                &format!("/v1/tickets/{t}/deps"),
                json!({ "blocked_by": dep }),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{body}");
    }

    fn node_ids(v: &Value) -> Vec<String> {
        let mut n: Vec<String> = v["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["id"].as_str().unwrap().to_string())
            .collect();
        n.sort();
        n
    }
    fn edge_set(v: &Value) -> Vec<(String, String)> {
        let mut e: Vec<(String, String)> = v["edges"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| {
                (
                    x["ticket"].as_str().unwrap().to_string(),
                    x["blocked_by"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        e.sort();
        e
    }
    fn sorted(mut v: Vec<String>) -> Vec<String> {
        v.sort();
        v
    }
    fn esorted(mut v: Vec<(String, String)>) -> Vec<(String, String)> {
        v.sort();
        v
    }

    // Direct (non-transitive) blocked_by on A: just A -> B.
    let (s, g) = app.get(&app.admin, &format!("/v1/tickets/{a}/deps")).await;
    assert_eq!(s, StatusCode::OK, "{g}");
    assert_eq!(g["direction"], "blocked_by");
    assert_eq!(g["transitive"], false);
    assert_eq!(node_ids(&g), sorted(vec![a.clone(), b.clone()]));
    assert_eq!(edge_set(&g), vec![(a.clone(), b.clone())]);

    // Transitive blocked_by on A: A -> B -> C.
    let (_, g) = app
        .get(&app.admin, &format!("/v1/tickets/{a}/deps?transitive=true"))
        .await;
    assert_eq!(node_ids(&g), sorted(vec![a.clone(), b.clone(), c.clone()]));
    assert_eq!(
        edge_set(&g),
        esorted(vec![(a.clone(), b.clone()), (b.clone(), c.clone())])
    );

    // Reverse (blocks) from C, direct: C is blocked_by B, i.e. C blocks B.
    let (_, g) = app
        .get(
            &app.admin,
            &format!("/v1/tickets/{c}/deps?direction=blocks"),
        )
        .await;
    assert_eq!(g["direction"], "blocks");
    assert_eq!(node_ids(&g), sorted(vec![c.clone(), b.clone()]));
    assert_eq!(edge_set(&g), vec![(b.clone(), c.clone())]);

    // Reverse transitive from C reaches A through B.
    let (_, g) = app
        .get(
            &app.admin,
            &format!("/v1/tickets/{c}/deps?direction=blocks&transitive=true"),
        )
        .await;
    assert_eq!(node_ids(&g), sorted(vec![a.clone(), b.clone(), c.clone()]));
    assert_eq!(
        edge_set(&g),
        esorted(vec![(a.clone(), b.clone()), (b.clone(), c.clone())])
    );

    // `both` transitive from the middle node B walks both ways and TERMINATES:
    // A blocks-edge points back to B and B blocked_by-edge back to A, so the
    // visited-set cycle guard is exercised. It must reach all three nodes with
    // exactly the two canonical edges and not loop.
    let (_, g) = app
        .get(
            &app.admin,
            &format!("/v1/tickets/{b}/deps?direction=both&transitive=true"),
        )
        .await;
    assert_eq!(g["direction"], "both");
    assert_eq!(node_ids(&g), sorted(vec![a.clone(), b.clone(), c.clone()]));
    assert_eq!(
        edge_set(&g),
        esorted(vec![(a.clone(), b.clone()), (b.clone(), c.clone())])
    );

    // include=deps carries `blocks` (direct reverse edges) alongside blocked_by.
    let (_, ct) = app
        .get(&app.admin, &format!("/v1/tickets/{c}?include=deps"))
        .await;
    let blocks: Vec<&str> = ct["deps"]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap())
        .collect();
    assert_eq!(blocks, vec![b.as_str()], "C blocks B: {ct}");
    assert!(
        ct["deps"]["blocked_by"].as_array().unwrap().is_empty(),
        "C is blocked by nothing: {ct}"
    );

    // Unknown direction -> 400.
    let (s, body) = app
        .get(
            &app.admin,
            &format!("/v1/tickets/{a}/deps?direction=sideways"),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "validation.direction");
}

#[tokio::test]
async fn archive_hides_from_default_views_and_migration_is_additive() {
    // --- Part 1: behaviour over HTTP -------------------------------------
    let app = TestApp::spawn().await;
    let keeper = app.create_ticket("Stays active").await;
    let archived = app.create_ticket("Will be archived").await;
    // Drive the soon-archived ticket to a claimable (ready) state so the
    // ready-queue exclusion is meaningful.
    app.to_ready(&archived).await;

    let ready_has = |v: &Value, id: &str| -> bool {
        v["items"].as_array().unwrap().iter().any(|t| t["id"] == id)
    };
    let list_has = |v: &Value, id: &str| -> bool {
        v["items"].as_array().unwrap().iter().any(|t| t["id"] == id)
    };

    // Before archiving: present in ready and counted in metrics.
    let (_, ready) = app.get(&app.admin, "/v1/ready?project=tp").await;
    assert!(ready_has(&ready, &archived), "ready should list it first");
    let (_, m0) = app.get(&app.admin, "/v1/metrics").await;
    let total0 = m0["projects"]["tp"]["total"].as_i64().unwrap();

    // Archive it (write scope).
    let (s, body) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{archived}/archive"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert!(body["archived_at"].is_string(), "archived_at set: {body}");

    // Default list excludes it; keeper still shows.
    let (_, list) = app.get(&app.admin, "/v1/tickets?project=tp").await;
    assert!(
        !list_has(&list, &archived),
        "archived hidden from default list"
    );
    assert!(list_has(&list, &keeper), "active ticket still listed");

    // archived=only and include_archived=true surface it.
    let (_, only) = app
        .get(&app.admin, "/v1/tickets?project=tp&archived=only")
        .await;
    assert!(list_has(&only, &archived) && !list_has(&only, &keeper));
    let (_, incl) = app
        .get(&app.admin, "/v1/tickets?project=tp&include_archived=true")
        .await;
    assert!(list_has(&incl, &archived) && list_has(&incl, &keeper));

    // Ready queue and metrics exclude it.
    let (_, ready) = app.get(&app.admin, "/v1/ready?project=tp").await;
    assert!(
        !ready_has(&ready, &archived),
        "archived excluded from ready"
    );
    let (_, m1) = app.get(&app.admin, "/v1/metrics").await;
    let total1 = m1["projects"]["tp"]["total"].as_i64().unwrap();
    assert_eq!(total1, total0 - 1, "metrics drops the archived ticket");

    // The single-ticket GET still returns it (archived is not deleted).
    let (s, one) = app
        .get(&app.admin, &format!("/v1/tickets/{archived}"))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(one["archived_at"].is_string());

    // Unarchive restores it to the default views.
    let (s, body) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{archived}/unarchive"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert!(body["archived_at"].is_null(), "archived_at cleared: {body}");
    let (_, ready) = app.get(&app.admin, "/v1/ready?project=tp").await;
    assert!(ready_has(&ready, &archived), "unarchived returns to ready");

    // --- Part 2: additive, non-destructive startup migration -------------
    // Build a database with the PRE-migration schema (no archived_at column)
    // and seed it, then open it with the current binary and prove the column
    // is added without disturbing any existing row.
    use rusqlite::params;
    use takomo::store::{ArchivedFilter, Store, TicketListFilter};

    // The exact pre-archived_at DDL for the tables the code touches.
    const OLD_SCHEMA: &str = r#"
    CREATE TABLE projects (id TEXT PRIMARY KEY, name TEXT NOT NULL, workflow_json TEXT NOT NULL, created_at INTEGER NOT NULL);
    CREATE TABLE workflow_states (project TEXT NOT NULL, state TEXT NOT NULL, category TEXT NOT NULL, claimable INTEGER NOT NULL DEFAULT 0, terminal INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (project, state));
    CREATE TABLE tickets (
      id TEXT PRIMARY KEY, project TEXT NOT NULL REFERENCES projects(id), type TEXT NOT NULL DEFAULT 'task',
      parent TEXT REFERENCES tickets(id), title TEXT NOT NULL, body TEXT NOT NULL DEFAULT '', state TEXT NOT NULL,
      priority TEXT NOT NULL DEFAULT 'normal', labels TEXT NOT NULL DEFAULT '[]', metadata TEXT NOT NULL DEFAULT '{}',
      links TEXT NOT NULL DEFAULT '{}', claim_holder TEXT, claim_expires_at INTEGER, fence_seq INTEGER NOT NULL DEFAULT 0,
      version INTEGER NOT NULL DEFAULT 1, created_by TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
    CREATE TABLE deps (ticket TEXT NOT NULL REFERENCES tickets(id), blocked_by TEXT NOT NULL REFERENCES tickets(id), PRIMARY KEY (ticket, blocked_by));
    CREATE TABLE comments (id TEXT PRIMARY KEY, ticket TEXT NOT NULL REFERENCES tickets(id), author TEXT NOT NULL, body TEXT NOT NULL, created_at INTEGER NOT NULL);
    CREATE TABLE events (seq INTEGER PRIMARY KEY AUTOINCREMENT, ticket TEXT, project TEXT, actor TEXT NOT NULL, kind TEXT NOT NULL, payload TEXT NOT NULL DEFAULT '{}', at INTEGER NOT NULL);
    CREATE TABLE tokens (id TEXT PRIMARY KEY, hash TEXT NOT NULL UNIQUE, actor TEXT NOT NULL, scopes TEXT NOT NULL, projects TEXT NOT NULL DEFAULT '*', rate_limit INTEGER NOT NULL DEFAULT 120, created_at INTEGER NOT NULL, expires_at INTEGER, revoked_at INTEGER, last_used_at INTEGER);
    CREATE TABLE idempotency (actor TEXT NOT NULL, key TEXT NOT NULL, ticket TEXT NOT NULL, created_at INTEGER NOT NULL, PRIMARY KEY (actor, key));
    "#;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("old.db");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(OLD_SCHEMA).unwrap();
        // Confirm the seed DB genuinely lacks the new column.
        let cols: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(tickets)").unwrap();
            stmt.query_map([], |r| r.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert!(
            !cols.iter().any(|c| c == "archived_at"),
            "seed DB should predate archived_at"
        );
        conn.execute(
            "INSERT INTO projects (id,name,workflow_json,created_at) VALUES ('op','Old Project','{}',1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO workflow_states (project,state,category,claimable,terminal) VALUES ('op','brief','todo',0,0),('op','done','done',0,1)",
            [],
        )
        .unwrap();
        for (id, state) in [("op-aaaa", "brief"), ("op-bbbb", "done")] {
            conn.execute(
                "INSERT INTO tickets (id,project,type,parent,title,body,state,priority,labels,metadata,links,fence_seq,version,created_by,created_at,updated_at) \
                 VALUES (?1,'op','task',NULL,?2,'legacy body',?3,'normal','[\"keep\"]','{\"x.k\":1}','{}',0,3,'seed',1,2)",
                params![id, format!("Legacy {id}"), state],
            )
            .unwrap();
        }
    }

    // Open with the current binary — runs the additive migration.
    let store = Store::open(&db_path).unwrap();

    // Every pre-existing ticket survived, unchanged, with archived_at defaulting
    // to null.
    let a = store
        .get_ticket("op-aaaa")
        .unwrap()
        .expect("legacy a survived");
    assert_eq!(a.title, "Legacy op-aaaa");
    assert_eq!(a.state, "brief");
    assert_eq!(a.body, "legacy body");
    assert_eq!(a.labels, vec!["keep".to_string()]);
    assert_eq!(a.version, 3, "existing version untouched");
    assert!(a.archived_at.is_none());
    let b = store
        .get_ticket("op-bbbb")
        .unwrap()
        .expect("legacy b survived");
    assert_eq!(b.state, "done");
    assert!(b.archived_at.is_none());

    // The new column is functional against the migrated DB.
    store.archive_ticket("op-bbbb", "test:mig").unwrap();
    let active = TicketListFilter {
        project: Some("op".into()),
        ..Default::default()
    };
    let (rows, _) = store.list_tickets(&active, None, 50).unwrap();
    let ids: Vec<&str> = rows.iter().map(|t| t.id.as_str()).collect();
    assert!(
        ids.contains(&"op-aaaa") && !ids.contains(&"op-bbbb"),
        "archived hidden after migration: {ids:?}"
    );
    let only = TicketListFilter {
        project: Some("op".into()),
        archived: ArchivedFilter::Only,
        ..Default::default()
    };
    let (arch_rows, _) = store.list_tickets(&only, None, 50).unwrap();
    assert_eq!(arch_rows.len(), 1);
    assert_eq!(arch_rows[0].id, "op-bbbb");

    // Nothing was dropped: both original rows are still present.
    let all = TicketListFilter {
        project: Some("op".into()),
        archived: ArchivedFilter::Include,
        ..Default::default()
    };
    let (all_rows, _) = store.list_tickets(&all, None, 50).unwrap();
    assert_eq!(all_rows.len(), 2, "no data lost in migration");

    // Migration is idempotent: opening the already-migrated DB again is a no-op
    // and the data is still intact.
    drop(store);
    let store2 = Store::open(&db_path).unwrap();
    assert!(store2.get_ticket("op-aaaa").unwrap().is_some());
    assert!(store2
        .get_ticket("op-bbbb")
        .unwrap()
        .unwrap()
        .archived_at
        .is_some());
}

// --- shareable read-only web views (takomo share) -------------------------------

// A project share lists exactly that project's tickets, and nothing from any
// other project. Also covers self-meta (workflow + scope) and per-ticket detail
// scoping (in-scope 200, out-of-scope 404).
#[tokio::test]
async fn share_project_scopes_to_that_project_only() {
    let app = TestApp::spawn().await;
    let t1 = app.create_ticket("tp ticket one").await;
    let _t2 = app.create_ticket("tp ticket two").await;

    // A second project with its own ticket must never leak into a tp share.
    let (s, _) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({ "id": "tp2", "name": "Second" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, other) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "tp2", "title": "other" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let other_id = other["id"].as_str().unwrap().to_string();

    let (s, share) = app
        .post(
            &app.admin,
            "/v1/shares",
            json!({ "kind": "project", "ref": "tp" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "create share: {share}");
    let token = share["token"].as_str().unwrap().to_string();
    assert!(token.starts_with("tks_"), "share token has its own prefix");
    assert_eq!(share["kind"], "project");
    assert_eq!(share["ref"], "tp");
    assert_eq!(share["path"], format!("/board#s={token}"));

    // self-meta carries the workflow so the board can render columns.
    let (s, meta) = app.get(&token, "/v1/shares/self").await;
    assert_eq!(s, StatusCode::OK, "self: {meta}");
    assert_eq!(meta["project"], "tp");
    assert_eq!(meta["kind"], "project");
    assert!(
        meta["workflow"]["states"].is_array(),
        "workflow present: {meta}"
    );

    let (s, list) = app.get(&token, "/v1/shares/self/tickets").await;
    assert_eq!(s, StatusCode::OK);
    let ids: Vec<&str> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 2, "exactly tp's two tickets: {list}");
    assert!(ids.contains(&t1.as_str()));
    assert!(
        !ids.contains(&other_id.as_str()),
        "other project must not leak into the share"
    );

    // per-ticket detail: in scope 200, out of scope 404.
    let (s, detail) = app
        .get(&token, &format!("/v1/shares/self/tickets/{t1}"))
        .await;
    assert_eq!(s, StatusCode::OK, "in-scope detail: {detail}");
    assert!(detail["comments"].is_array());
    assert!(detail["deps"].is_object());
    let (s, _d) = app
        .get(&token, &format!("/v1/shares/self/tickets/{other_id}"))
        .await;
    assert_eq!(
        s,
        StatusCode::NOT_FOUND,
        "out-of-scope ticket is invisible to the share"
    );
}

// An epic (subtree) share lists exactly the root plus its full recursive
// descendant subtree — not siblings, not the rest of the project.
#[tokio::test]
async fn share_epic_scopes_to_subtree_only() {
    let app = TestApp::spawn().await;
    let epic = app.create_typed("Epic root", "epic", None).await;
    let c1 = app.create_typed("child one", "task", Some(&epic)).await;
    let c2 = app.create_typed("child two", "task", Some(&epic)).await;
    let g = app.create_typed("grandchild", "task", Some(&c1)).await;
    let sibling = app.create_typed("unrelated sibling", "task", None).await;

    let (s, share) = app
        .post(
            &app.admin,
            "/v1/shares",
            json!({ "kind": "epic", "ref": epic }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{share}");
    let token = share["token"].as_str().unwrap().to_string();
    // 'epic' is the caller-facing spelling; the stored/echoed kind is 'subtree'.
    assert_eq!(share["kind"], "subtree");
    assert_eq!(share["ref"], epic);

    let (s, list) = app.get(&token, "/v1/shares/self/tickets").await;
    assert_eq!(s, StatusCode::OK);
    let ids: Vec<String> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect();
    assert!(ids.contains(&epic), "root included");
    assert!(ids.contains(&c1), "direct child included");
    assert!(ids.contains(&c2), "direct child included");
    assert!(ids.contains(&g), "recursive descendant included");
    assert!(!ids.contains(&sibling), "sibling excluded from subtree");
    assert_eq!(ids.len(), 4, "exactly the subtree: {list}");

    // The sibling is out of scope for the per-ticket detail too.
    let (s, _d) = app
        .get(&token, &format!("/v1/shares/self/tickets/{sibling}"))
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

// A subtree share whose stored `kind` the store cannot interpret is refused
// outright — it must NOT fall back to the whole project.
//
// This is the fail-open direction the store used to have: the scope was chosen by
// `kind == "subtree"` on a `&str`, so anything else took the project-wide query.
// `epic` is the realistic wrong value, being the caller-facing spelling that
// `ShareKind::parse_request` accepts and normalizes away — one future code path
// storing the request spelling verbatim would have turned a share of one epic
// into a share of everything in the project, on a link designed to be pasted
// around. Only `ShareKind::as_str` can write the column now and only its own
// spellings read back, so the row below is unreachable through the API and has to
// be forced in.
#[tokio::test]
async fn share_with_uninterpretable_kind_fails_closed_not_project_wide() {
    let app = TestApp::spawn().await;
    let epic = app.create_typed("Epic root", "epic", None).await;
    let child = app
        .create_typed("child of the epic", "task", Some(&epic))
        .await;
    let outsider = app.create_typed("outside the subtree", "task", None).await;

    let (s, share) = app
        .post(
            &app.admin,
            "/v1/shares",
            json!({ "kind": "epic", "ref": epic }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{share}");
    let token = share["token"].as_str().unwrap().to_string();
    let share_id = share["id"].as_str().unwrap().to_string();

    // Baseline: the share is the subtree, and the outsider is already invisible.
    let (s, list) = app.get(&token, "/v1/shares/self/tickets").await;
    assert_eq!(s, StatusCode::OK, "{list}");
    let ids: Vec<String> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids, vec![epic.clone(), child.clone()], "baseline subtree");

    // Now the row a future caller-facing-spelling bug would leave behind.
    app.force_share_kind(&share_id, "epic");

    let (status, body) = app.get(&token, "/v1/shares/self/tickets").await;
    assert!(
        !body.to_string().contains(&outsider),
        "a share whose kind cannot be interpreted must not widen to the whole project: {body}"
    );
    assert!(
        !status.is_success(),
        "an uninterpretable share kind must be refused, not served: {status} {body}"
    );
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["code"], "share.kind_unrecognized");
    assert!(
        body["remedy"].as_str().is_some_and(|r| !r.is_empty()),
        "the refusal teaches how to recover: {body}"
    );

    // The refusal happens on the share auth path, so every `self*` endpoint is
    // closed — including the detail endpoint for a ticket inside the subtree.
    let (s, body) = app.get(&token, "/v1/shares/self").await;
    assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert_eq!(body["code"], "share.kind_unrecognized");
    let (s, body) = app
        .get(&token, &format!("/v1/shares/self/tickets/{child}"))
        .await;
    assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert_eq!(body["code"], "share.kind_unrecognized");
}

// A share token is read-only and reaches ONLY the share endpoints: it is
// rejected on every normal endpoint (read and write). A normal token is likewise
// not accepted as a share token.
#[tokio::test]
async fn share_token_rejected_on_normal_endpoints() {
    let app = TestApp::spawn().await;
    let _ = app.create_ticket("t").await;
    let (_, share) = app
        .post(
            &app.admin,
            "/v1/shares",
            json!({ "kind": "project", "ref": "tp" }),
        )
        .await;
    let token = share["token"].as_str().unwrap().to_string();

    // normal read endpoint
    let (s, _) = app.get(&token, "/v1/tickets?project=tp").await;
    assert_eq!(
        s,
        StatusCode::UNAUTHORIZED,
        "share token must not read arbitrary endpoints"
    );
    // normal write endpoint
    let (s, _) = app
        .post(
            &token,
            "/v1/tickets",
            json!({ "project": "tp", "title": "x" }),
        )
        .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "share token must not write");
    // whoami (any-valid-token endpoint) still rejects a share token
    let (s, _) = app.get(&token, "/v1/whoami").await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // A normal token is not a share token on the share path.
    let (s, _) = app.get(&app.admin, "/v1/shares/self").await;
    assert_eq!(
        s,
        StatusCode::UNAUTHORIZED,
        "normal token is not a share token"
    );
}

// Revocation is immediate: the share token then returns 410 Gone. List never
// discloses the plaintext token or its hash.
#[tokio::test]
async fn share_revocation_returns_410() {
    let app = TestApp::spawn().await;
    let _ = app.create_ticket("t").await;
    let (_, share) = app
        .post(
            &app.admin,
            "/v1/shares",
            json!({ "kind": "project", "ref": "tp" }),
        )
        .await;
    let token = share["token"].as_str().unwrap().to_string();
    let id = share["id"].as_str().unwrap().to_string();

    let (s, _) = app.get(&token, "/v1/shares/self").await;
    assert_eq!(s, StatusCode::OK, "share works before revoke");

    // list (admin) shows metadata but never the secret.
    let (s, ls) = app.get(&app.admin, "/v1/shares").await;
    assert_eq!(s, StatusCode::OK);
    let rows = ls.as_array().unwrap();
    assert!(rows.iter().any(|x| x["id"] == id));
    assert!(
        rows.iter()
            .all(|x| x.get("token").is_none() && x.get("token_hash").is_none()),
        "list must not disclose the token or its hash: {ls}"
    );

    let (s, _) = app.delete(&app.admin, &format!("/v1/shares/{id}")).await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    let (s, body) = app.get(&token, "/v1/shares/self").await;
    assert_eq!(s, StatusCode::GONE, "revoked share is gone: {body}");
    assert_eq!(body["code"], "share.expired");
    let (s, _) = app.get(&token, "/v1/shares/self/tickets").await;
    assert_eq!(s, StatusCode::GONE);
}

// An expired share returns 410 Gone; a still-valid one works. Uses a backdated
// mint (a second DB connection) so the test does not sleep on wall-clock time.
#[tokio::test]
async fn share_expiry_returns_410() {
    let app = TestApp::spawn().await;
    let _ = app.create_ticket("t").await;
    let store = app.open_store();

    // expires_at in the far past -> already expired.
    let (_, expired) = store
        .create_share(ShareKind::Project, "tp", "tp", 1, "test:setup")
        .unwrap();
    let (s, body) = app.get(&expired, "/v1/shares/self").await;
    assert_eq!(s, StatusCode::GONE, "expired share is gone: {body}");
    assert_eq!(body["code"], "share.expired");

    // a future expiry still works.
    let future = takomo::ids::now_ms() + 60_000;
    let (_, fresh) = store
        .create_share(ShareKind::Project, "tp", "tp", future, "test:setup")
        .unwrap();
    let (s, _) = app.get(&fresh, "/v1/shares/self").await;
    assert_eq!(s, StatusCode::OK, "unexpired share works");
}

// Archived tickets are excluded from a share by default and included on request.
#[tokio::test]
async fn share_excludes_archived_by_default() {
    let app = TestApp::spawn().await;
    let keep = app.create_ticket("active ticket").await;
    let gone = app.create_ticket("to be archived").await;
    let (s, _) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{gone}/archive"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "archive");

    let (_, share) = app
        .post(
            &app.admin,
            "/v1/shares",
            json!({ "kind": "project", "ref": "tp" }),
        )
        .await;
    let token = share["token"].as_str().unwrap().to_string();

    let (s, list) = app.get(&token, "/v1/shares/self/tickets").await;
    assert_eq!(s, StatusCode::OK);
    let ids: Vec<&str> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&keep.as_str()));
    assert!(
        !ids.contains(&gone.as_str()),
        "archived excluded by default"
    );

    let (s, list) = app
        .get(&token, "/v1/shares/self/tickets?include_archived=true")
        .await;
    assert_eq!(s, StatusCode::OK);
    let ids: Vec<&str> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&gone.as_str()), "archived included on request");
}

/// Mint a project share over `tp` and return its token.
async fn tp_share(app: &TestApp) -> String {
    let (s, share) = app
        .post(
            &app.admin,
            "/v1/shares",
            json!({ "kind": "project", "ref": "tp" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "create share: {share}");
    share["token"].as_str().expect("share token").to_string()
}

/// takomo-vlpm / takomo-fgca, the store half: `share_tickets` was an unbounded
/// full-project scan, and a share link is a credential designed to be pasted
/// around — so one request could be made to walk the whole project, repeatedly.
///
/// It is a page now, and the page is the interesting part twice over: it is
/// *bounded* whatever the caller asks for, and it never truncates silently. A
/// viewer shown 200 of 250 tickets with nothing saying so would be worse than a
/// slow board.
#[tokio::test]
async fn share_tickets_are_paged_and_bounded() {
    let app = TestApp::spawn().await;
    let named = app.create_ticket("A ticket with a name").await;
    // 250 more, so the scope is larger than one page whatever the caller asks for.
    app.seed_bulk_tickets(250);
    let token = tp_share(&app).await;

    // No `limit`: the default page, not the whole project.
    let (s, page) = app.get(&token, "/v1/shares/self/tickets").await;
    assert_eq!(s, StatusCode::OK, "{page}");
    let first: Vec<String> = page["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|t| t["id"].as_str().expect("id").to_string())
        .collect();
    assert_eq!(first.len(), 200, "one page, not 251 tickets");
    assert!(
        first.contains(&named),
        "the page starts at the oldest ticket: {}",
        first[0]
    );

    // And it says so — in a cursor for a client and in words for a human.
    let cursor = page["next_cursor"]
        .as_str()
        .unwrap_or_else(|| panic!("a truncated page must carry next_cursor: {page}"))
        .to_string();
    let warning = page["warning"]
        .as_str()
        .unwrap_or_else(|| panic!("a truncated page must say so: {page}"));
    assert!(
        warning.contains("not the whole share") && warning.contains(&format!("cursor={cursor}")),
        "the warning must name the call that fetches the rest: {warning}"
    );

    // An over-large `limit` is clamped to the same ceiling rather than honoured.
    let (s, greedy) = app
        .get(&token, "/v1/shares/self/tickets?limit=100000")
        .await;
    assert_eq!(s, StatusCode::OK, "{greedy}");
    assert_eq!(
        greedy["items"].as_array().expect("items").len(),
        200,
        "the page ceiling is not negotiable: {greedy}"
    );

    // Paging through reaches every ticket exactly once and then stops cleanly.
    let mut seen = first.clone();
    let mut next = Some(cursor);
    while let Some(c) = next {
        let (s, page) = app
            .get(&token, &format!("/v1/shares/self/tickets?cursor={c}"))
            .await;
        assert_eq!(s, StatusCode::OK, "{page}");
        for t in page["items"].as_array().expect("items") {
            seen.push(t["id"].as_str().expect("id").to_string());
        }
        next = page["next_cursor"].as_str().map(str::to_string);
        if next.is_none() {
            assert!(
                page.get("warning").is_none(),
                "the last page must not warn about more: {page}"
            );
        }
    }
    assert_eq!(seen.len(), 251, "every ticket in scope, once");
    let unique: std::collections::HashSet<&String> = seen.iter().collect();
    assert_eq!(unique.len(), 251, "no ticket served twice across pages");

    // A cursor that is not a cursor is a teaching 400, not an empty board.
    let (s, bad) = app
        .get(&token, "/v1/shares/self/tickets?cursor=not-a-number")
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}");
    assert_eq!(bad["code"], "validation.cursor");
}

/// takomo-fgca, the auth half: nothing rate-limited the share path at all. The
/// `tk_` budget charges writes only — right for a named, revocable actor whose risk
/// is a runaway write loop — but a `tks_` link is a read-only bearer capability
/// meant to be pasted around, so reads are its entire attack surface and it had no
/// budget of any kind.
///
/// The budget is per link, which is also what makes it the right control: the link
/// is the identity, and revoking it is the mitigation an operator has.
#[tokio::test]
async fn share_requests_are_rate_limited_per_link() {
    let app = TestApp::spawn().await;
    let _ = app.create_ticket("t").await;
    let hammered = tp_share(&app).await;
    let bystander = tp_share(&app).await;

    // 120 requests/minute is the budget; the 121st inside the window is refused.
    let mut refused = None;
    for i in 0..130 {
        let resp = app
            .authed(Method::GET, &hammered, "/v1/shares/self")
            .send()
            .await
            .expect("request");
        if resp.status() == StatusCode::TOO_MANY_REQUESTS {
            refused = Some((i, resp));
            break;
        }
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "request {i} should have been served"
        );
    }
    let (at, resp) = refused.expect("the share path must be rate limited at all");
    assert_eq!(at, 120, "the budget is 120 requests in the window");

    let retry_after: i64 = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| panic!("a 429 must carry Retry-After: {:?}", resp.headers()));
    assert!(
        (1..=60).contains(&retry_after),
        "Retry-After must be inside the window: {retry_after}"
    );
    let body: Value = resp.json().await.expect("json body");
    assert_eq!(body["code"], "share.rate_limited");
    assert!(
        body["message"]
            .as_str()
            .expect("message")
            .contains("still works"),
        "a viewer must be told the link is throttled, not broken: {body}"
    );
    assert!(
        body["remedy"]
            .as_str()
            .is_some_and(|r| r.contains("revoke")),
        "the remedy names what the owner can do about a leaked link: {body}"
    );

    // The tickets route is charged from the same window: the throttle is on the
    // link, not on one endpoint.
    let (s, also) = app.get(&hammered, "/v1/shares/self/tickets").await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS, "{also}");

    // A different link is untouched — one leaked share must not take the others
    // down with it, which a single global counter would have done.
    let (s, fine) = app.get(&bystander, "/v1/shares/self").await;
    assert_eq!(
        s,
        StatusCode::OK,
        "the budget is per share, not per server: {fine}"
    );
}

// Share creation validates the referent and enforces write scope + project scope.
#[tokio::test]
async fn share_creation_validates_ref_and_authority() {
    let app = TestApp::spawn().await;
    let _ = app.create_ticket("t").await;

    // unknown project / ticket -> 404.
    let (s, _) = app
        .post(
            &app.admin,
            "/v1/shares",
            json!({ "kind": "project", "ref": "nope" }),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = app
        .post(
            &app.admin,
            "/v1/shares",
            json!({ "kind": "epic", "ref": "tp-zzzz" }),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // bad kind / over-cap ttl -> 422.
    let (s, _) = app
        .post(
            &app.admin,
            "/v1/shares",
            json!({ "kind": "bogus", "ref": "tp" }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, _) = app
        .post(
            &app.admin,
            "/v1/shares",
            json!({ "kind": "project", "ref": "tp", "ttl_seconds": 99_999_999 }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);

    // a read-only token cannot mint a share (needs write scope).
    let readonly = app.mint("agent:ro", &["read"], None);
    let (s, _) = app
        .post(
            &readonly,
            "/v1/shares",
            json!({ "kind": "project", "ref": "tp" }),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "share mint needs write scope");

    // a token scoped to a different project cannot share tp.
    let scoped = app.mint("agent:elsewhere", &["read", "write"], Some(&["other"]));
    let (s, _) = app
        .post(
            &scoped,
            "/v1/shares",
            json!({ "kind": "project", "ref": "tp" }),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "cannot share a project outside token scope"
    );
}

// ---------------------------------------------------------------------------
// Ask-a-human board

#[tokio::test]
async fn question_ask_parks_ticket_and_answer_resumes_it() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Delete the legacy billing table?").await;
    let fence = app.to_implementing(&id).await;

    // Agent asks a confirm question, echoing its lease fence.
    let (qid, body) = app
        .ask(
            &app.worker,
            json!({
                "ticket": id,
                "kind": "confirm",
                "title": "OK to drop table billing_v1?",
                "body": "It has no reads in 90d but I want a human to confirm.",
                "expertise": ["domain:billing"],
                "urgency": "high",
                "fence": fence,
            }),
        )
        .await;
    // Ticket is parked in the blocked state and the lease was released.
    assert_eq!(body["ticket"]["state"], "needs-decision");
    assert!(
        body["ticket"]["claim"].is_null(),
        "lease should be released"
    );

    // The inbox shows it as open, routable by expertise.
    let (s, list) = app
        .get(&app.human, "/v1/questions?project=tp&status=open")
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(list["items"].as_array().unwrap().len(), 1);
    assert_eq!(list["items"][0]["id"], qid);

    // A token without the human scope cannot answer.
    let (s, denied) = app.answer(&app.worker, &qid, json!("yes")).await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "worker answered without human scope: {denied}"
    );
    assert_eq!(denied["code"], "auth.scope");

    // The human answers yes; the ticket resumes into the claimable ready state.
    let (s, answered) = app
        .answer(
            &app.human,
            &qid,
            json!({ "value": "yes", "note": "confirmed with data team" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "answer failed: {answered}");
    assert_eq!(answered["question"]["status"], "answered");
    assert_eq!(answered["question"]["answer"]["value"], true);
    assert_eq!(answered["question"]["resolved_to"], "ready");
    assert_eq!(answered["ticket"]["state"], "ready");

    // The exchange is recorded as a comment the resuming agent can read. It
    // leads with the decision and names the question by id — restating the
    // question TITLE would only repeat what the reader already has above it.
    let (s, detail) = app
        .get(&app.worker, &format!("/v1/tickets/{id}?include=comments"))
        .await;
    assert_eq!(s, StatusCode::OK);
    let comments = detail["comments"].as_array().unwrap();
    let answer_comment = comments
        .iter()
        .find(|c| c["author"] == "human:reviewer")
        .and_then(|c| c["body"].as_str())
        .unwrap_or_else(|| panic!("answer should leave a comment: {detail}"));
    assert_eq!(
        answer_comment,
        format!("Human answered {qid}: yes / approved — confirmed with data team"),
        "answer comment shape changed"
    );
    assert!(
        !answer_comment.contains("OK to drop table billing_v1?"),
        "the comment must not restate the question title: {answer_comment}"
    );

    // Answering again is rejected — the question is closed.
    let (s, again) = app.answer(&app.human, &qid, json!("no")).await;
    assert_eq!(s, StatusCode::CONFLICT, "{again}");
    assert_eq!(again["code"], "question.not_open");
}

#[tokio::test]
async fn question_followup_loop_bounces_to_agent_and_back_before_answering() {
    let app = TestApp::spawn().await;
    let id = app
        .create_ticket("Prod schema migration for billing_rollup")
        .await;
    let fence = app.to_implementing(&id).await;

    // Agent asks a blocking approve question; the ticket parks.
    let (qid, body) = app
        .ask(
            &app.worker,
            json!({
                "ticket": id,
                "kind": "confirm",
                "title": "Run the additive migration on prod now?",
                "expertise": ["domain:billing"],
                "urgency": "high",
                "fence": fence,
            }),
        )
        .await;
    assert_eq!(
        body["question"]["awaiting"], "human",
        "fresh question awaits a human"
    );
    assert_eq!(body["ticket"]["state"], "needs-decision");

    // An agent can't reply out of turn: nothing has been bounced back yet.
    let (s, oot) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{qid}/reply"),
            json!({ "message": "unsolicited" }),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "out-of-turn reply should be refused: {oot}"
    );
    assert_eq!(oot["code"], "question.not_awaiting_reply");

    // Human bounces it back for more research instead of answering.
    let (s, fu) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/followup"),
            json!({ "message": "What's the row count and lock time on prod?" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "followup failed: {fu}");
    assert_eq!(
        fu["status"], "open",
        "question stays open during a follow-up"
    );
    assert_eq!(fu["awaiting"], "agent", "now the agent owes a reply");

    // The ticket stays parked — the human still owns the eventual answer.
    let (_, tk) = app.get(&app.worker, &format!("/v1/tickets/{id}")).await;
    assert_eq!(
        tk["state"], "needs-decision",
        "ticket stays parked mid-thread"
    );

    // Thread now carries the human's request.
    let (_, detail) = app.get(&app.worker, &format!("/v1/questions/{qid}")).await;
    let thread = detail["thread"].as_array().unwrap();
    assert_eq!(thread.len(), 1);
    assert_eq!(thread[0]["role"], "human");

    // The asking agent replies with the research; the thread returns to the human.
    let (s, rep) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{qid}/reply"),
            json!({ "message": "40k rows, ~2s lock, fully reversible." }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "reply failed: {rep}");
    assert_eq!(rep["awaiting"], "human", "back to the human to decide");

    let (_, detail2) = app.get(&app.human, &format!("/v1/questions/{qid}")).await;
    assert_eq!(detail2["thread"].as_array().unwrap().len(), 2);

    // Both turns are mirrored onto the ticket, and both name the question by id
    // rather than restating its title — the title is repetition wherever the
    // question is on screen, and the id resolves to the whole question wherever
    // it is not.
    let (_, mirror) = app
        .get(&app.worker, &format!("/v1/tickets/{id}?include=comments"))
        .await;
    let bodies: Vec<&str> = mirror["comments"]
        .as_array()
        .expect("comments")
        .iter()
        .filter_map(|c| c["body"].as_str())
        .collect();
    assert!(
        bodies.iter().any(|b| *b
            == format!(
                "Human asked agent:w1 for more before answering {qid}: What's the row count and lock time on prod?"
            )),
        "follow-up comment shape changed: {bodies:?}"
    );
    assert!(
        bodies
            .iter()
            .any(|b| *b
                == format!("agent:w1 replied on {qid}: 40k rows, ~2s lock, fully reversible.")),
        "reply comment shape changed: {bodies:?}"
    );
    assert!(
        !bodies
            .iter()
            .any(|b| b.contains("Run the additive migration on prod now?")),
        "no mirrored comment may restate the question title: {bodies:?}"
    );

    // Now the human answers; the ticket resumes.
    let (s, answered) = app.answer(&app.human, &qid, json!("yes")).await;
    assert_eq!(s, StatusCode::OK, "answer failed: {answered}");
    assert_eq!(answered["question"]["status"], "answered");
    assert_eq!(answered["ticket"]["state"], "ready");

    // A follow-up on a closed question is refused.
    let (s, closed) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/followup"),
            json!({ "message": "too late?" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{closed}");
    assert_eq!(closed["code"], "question.not_open");
}

/// Revising a still-open choose question's options: the point is that an agent
/// which learns something mid-thread can fix the choices instead of withdrawing
/// the question and losing the thread. The revision must leave the question
/// coherent (no recommendation pointing at a removed option), must not touch
/// whose turn it is, and must be refused once the question is settled.
#[tokio::test]
async fn question_options_can_be_revised_while_open() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Pick a cache eviction policy").await;
    let fence = app.to_implementing(&id).await;

    let (qid, _) = app
        .ask(
            &app.worker,
            json!({
                "ticket": id,
                "kind": "choose",
                "title": "Which eviction policy?",
                "options": ["LRU", "LFU", "FIFO"],
                "recommended": "LFU",
                "fence": fence,
            }),
        )
        .await;

    // Reword/extend while keeping the recommended option: recommendation stands,
    // and whose turn it is must not move.
    let (s, rev) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{qid}/options"),
            json!({
                "options": ["LRU", "LFU", "ARC"],
                "reason": "FIFO thrashes on our access pattern; ARC is the real contender",
            }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "revise failed: {rev}");
    assert_eq!(rev["options"], json!(["LRU", "LFU", "ARC"]));
    assert_eq!(rev["recommended"], "LFU", "still a valid option, so kept");
    assert_eq!(rev["awaiting"], "human", "revising is not a turn change");
    assert_eq!(rev["status"], "open");

    // Dropping the recommended option without naming a new one is refused —
    // a dangling recommendation must not be silently dropped.
    let (s, dangling) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{qid}/options"),
            json!({ "options": ["LRU", "ARC"] }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{dangling}");
    assert_eq!(dangling["code"], "validation.recommended");
    assert!(
        dangling["message"].as_str().unwrap().contains("LFU"),
        "the error must name the stale recommendation: {dangling}"
    );

    // Same revision, now naming the new recommendation.
    let (s, rev2) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{qid}/options"),
            json!({ "options": ["LRU", "ARC"], "recommended": "ARC", "recommended_note": "adaptive" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{rev2}");
    assert_eq!(rev2["recommended"], "ARC");
    assert_eq!(rev2["recommended_note"], "adaptive");

    // Explicit null clears the recommendation.
    let (s, cleared) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{qid}/options"),
            json!({ "options": ["LRU", "ARC"], "recommended": null }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{cleared}");
    assert!(cleared["recommended"].is_null(), "{cleared}");

    // Below two options it is no longer a choice.
    let (s, one) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{qid}/options"),
            json!({ "options": ["LRU"] }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{one}");
    assert_eq!(one["code"], "validation.options");

    // The audit trail: a ticket comment naming the change, and an event.
    let (s, t) = app
        .get(&app.worker, &format!("/v1/tickets/{id}?include=comments"))
        .await;
    assert_eq!(s, StatusCode::OK);
    let comments = t["comments"].as_array().unwrap();
    assert!(
        comments.iter().any(|c| {
            let b = c["body"].as_str().unwrap_or("");
            // The comment carries the before/after sets and the stated reason, so
            // a human who read the old options can see exactly what moved.
            b.contains(&format!("revised the options on {qid}"))
                && b.contains("LRU, LFU, FIFO")
                && b.contains("FIFO thrashes on our access pattern")
        }),
        "expected a revision comment naming the change: {comments:?}"
    );
    // The question is named by id; its title is not restated.
    assert!(
        !comments.iter().any(|c| c["body"]
            .as_str()
            .unwrap_or("")
            .contains("Which eviction policy?")),
        "revision comments must not restate the question title: {comments:?}"
    );

    // A human holding the ORIGINAL option list cannot land a stale pick.
    let (s, stale) = app.answer(&app.human, &qid, json!("FIFO")).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{stale}");
    assert_eq!(stale["code"], "validation.answer");

    // Answer it for real, then revising is refused: the choices a decision was
    // made on must stay on the record.
    let (s, answered) = app.answer(&app.human, &qid, json!("ARC")).await;
    assert_eq!(s, StatusCode::OK, "{answered}");
    let (s, settled) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{qid}/options"),
            json!({ "options": ["LRU", "LFU"] }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{settled}");
    assert_eq!(settled["code"], "question.not_open");
}

/// Revising options while an outsider holds a single-use answer grant: the grant
/// view must show the current options, and a stale pick is refused (takomo-hms9).
#[tokio::test]
async fn revising_options_invalidates_stale_answer_grant_picks() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Grant holder sees revised options").await;
    let fence = app.to_implementing(&id).await;
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({
                "ticket": id,
                "kind": "choose",
                "title": "Which region?",
                "options": ["us-east", "eu-west"],
                "fence": fence,
            }),
        )
        .await;

    let (s, link) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/answer-link"),
            json!({ "actor": "human:contractor" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{link}");
    let token = link["token"].as_str().unwrap();

    let (s, rev) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{qid}/options"),
            json!({
                "options": ["us-east", "ap-south"],
                "reason": "eu-west is not in scope for this rollout",
            }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{rev}");
    assert_eq!(rev["options"], json!(["us-east", "ap-south"]));

    let (s, view) = app.get(token, "/v1/answer/self").await;
    assert_eq!(s, StatusCode::OK, "{view}");
    assert_eq!(view["question"]["options"], json!(["us-east", "ap-south"]));

    let (s, stale) = app
        .post(token, "/v1/answer/self", json!({ "answer": "eu-west" }))
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{stale}");
    assert_eq!(stale["code"], "validation.answer");

    let (s, answered) = app
        .post(token, "/v1/answer/self", json!({ "answer": "ap-south" }))
        .await;
    assert_eq!(s, StatusCode::OK, "{answered}");
    assert_eq!(answered["question"]["status"], "answered");
}

/// Options only exist on a `choose` question, so revising any other kind is a
/// validation error rather than a silent no-op.
#[tokio::test]
async fn question_options_revision_rejects_non_choose_kinds() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Drop the legacy table?").await;
    let fence = app.to_implementing(&id).await;
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "Drop it?", "fence": fence }),
        )
        .await;
    let (s, err) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{qid}/options"),
            json!({ "options": ["yes", "no"] }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
    assert_eq!(err["code"], "validation.kind");
    assert!(
        err["message"].as_str().unwrap().contains("confirm"),
        "{err}"
    );
}

#[tokio::test]
async fn question_multi_select_choose_round_trip_and_answer() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Enable regions").await;
    let fence = app.to_implementing(&id).await;

    let (qid, body) = app
        .ask(
            &app.worker,
            json!({
                "ticket": id,
                "kind": "choose",
                "multi": true,
                "title": "Which regions should launch?",
                "options": ["US", "EU", "UK", "APAC"],
                "recommended_multi": ["US", "EU"],
                "fence": fence,
            }),
        )
        .await;
    assert_eq!(body["question"]["multi"], true);
    assert_eq!(body["question"]["recommended_multi"], json!(["US", "EU"]));

    // Answer with a subset array.
    let (s, ans) = app.answer(&app.human, &qid, json!(["US", "APAC"])).await;
    assert_eq!(s, StatusCode::OK, "multi answer failed: {ans}");
    assert_eq!(ans["question"]["status"], "answered");
    assert_eq!(ans["question"]["answer"]["value"], json!(["US", "APAC"]));
    assert_eq!(ans["ticket"]["state"], "ready");

    // A non-array answer to a multi question is refused.
    let id2 = app.create_ticket("Regions 2").await;
    let fence2 = app.to_implementing(&id2).await;
    let (_, b2) = app
        .post(
            &app.worker,
            "/v1/questions",
            json!({ "ticket": id2, "kind": "choose", "multi": true, "title": "?", "options": ["A", "B"], "fence": fence2 }),
        )
        .await;
    let qid2 = b2["question"]["id"].as_str().unwrap().to_string();
    let (s, bad) = app.answer(&app.human, &qid2, json!("A")).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");

    // multi on a non-choose kind is refused at ask time.
    let id3 = app.create_ticket("Regions 3").await;
    let fence3 = app.to_implementing(&id3).await;
    let (s, bad3) = app
        .post(
            &app.worker,
            "/v1/questions",
            json!({ "ticket": id3, "kind": "confirm", "multi": true, "title": "x", "fence": fence3 }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{bad3}");
    assert_eq!(bad3["code"], "validation.multi");
}

#[tokio::test]
async fn question_multi_rejects_a_string_instead_of_a_boolean() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("multi type").await;
    let fence = app.to_implementing(&id).await;
    let (s, body) = app
        .post(
            &app.worker,
            "/v1/questions",
            json!({
                "ticket": id,
                "kind": "choose",
                "title": "Regions?",
                "options": ["A", "B"],
                "multi": "true",
                "fence": fence,
            }),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "validation.field_type");
}

#[tokio::test]
async fn question_reopen_takes_back_answer_until_the_ticket_is_in_use() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Prod schema migration").await;
    let fence = app.to_implementing(&id).await;

    // Ask + answer: the ticket resumes into 'ready'.
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "Run it?", "fence": fence }),
        )
        .await;
    let (s, ans) = app.answer(&app.human, &qid, json!("yes")).await;
    assert_eq!(s, StatusCode::OK, "{ans}");
    assert_eq!(ans["question"]["status"], "answered");
    assert_eq!(ans["ticket"]["state"], "ready");

    // Reopen while the ticket is still free: the question returns to open and the
    // ticket is re-parked in a blocked state.
    let (s, re) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/reopen"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "reopen failed: {re}");
    assert_eq!(re["question"]["status"], "open");
    assert_eq!(re["question"]["answer"], serde_json::Value::Null);
    assert_eq!(re["ticket"]["state"], "needs-decision", "ticket re-parked");

    // The reopen note on the ticket names the question by id, like every other
    // question-driven comment, instead of restating its title.
    let (_, reopened_ticket) = app
        .get(&app.human, &format!("/v1/tickets/{id}?include=comments"))
        .await;
    let reopen_bodies: Vec<&str> = reopened_ticket["comments"]
        .as_array()
        .expect("comments")
        .iter()
        .filter_map(|c| c["body"].as_str())
        .collect();
    assert!(
        reopen_bodies
            .iter()
            .any(|b| *b
                == format!("human:reviewer reopened {qid} — parked again pending a new answer.")),
        "reopen comment shape changed: {reopen_bodies:?}"
    );
    assert!(
        !reopen_bodies.iter().any(|b| b.contains("Run it?")),
        "question-driven comments must not restate the question title: {reopen_bodies:?}"
    );

    // A read-only/write token without human scope can't reopen.
    let (s, _) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{qid}/reopen"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "reopen needs the human scope");

    // Answer again, then claim the resumed ticket → reopen is now refused because
    // a worker relies on the answer.
    let (_, ans2) = app.answer(&app.human, &qid, json!("yes")).await;
    assert_eq!(ans2["ticket"]["state"], "ready");
    app.claim(&id).await;
    let (s, blocked) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/reopen"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{blocked}");
    assert_eq!(blocked["code"], "question.reopen_claimed");
}

#[tokio::test]
async fn ticket_promote_records_history_and_project_index() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Ship the billing rollup").await;

    // Promote to staging, then production (free-form targets).
    let (s, p1) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/promote"),
            json!({ "target": "staging", "url": "https://ci/deploy/1", "ref": "v1.2.0" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "promote failed: {p1}");
    assert_eq!(p1["target"], "staging");
    assert_eq!(p1["ref"], "v1.2.0");

    let (s, _p2) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/promote"),
            json!({ "target": "production", "note": "canary then full" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);

    // History is newest-first.
    let (s, hist) = app
        .get(&app.worker, &format!("/v1/tickets/{id}/promotions"))
        .await;
    assert_eq!(s, StatusCode::OK);
    let items = hist["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["target"], "production", "newest first");
    assert_eq!(items[1]["target"], "staging");

    // The project index returns the latest per ticket (production).
    let (s, idx) = app.get(&app.worker, "/v1/promotions?project=tp").await;
    assert_eq!(s, StatusCode::OK);
    let idx_items = idx["items"].as_array().unwrap();
    assert_eq!(idx_items.len(), 1, "one row per ticket");
    assert_eq!(idx_items[0]["ticket"], id);
    assert_eq!(idx_items[0]["target"], "production");

    // include=promotions attaches the history to ticket detail.
    let (_, detail) = app
        .get(&app.worker, &format!("/v1/tickets/{id}?include=promotions"))
        .await;
    assert_eq!(detail["promotions"].as_array().unwrap().len(), 2);

    // An empty target is rejected.
    let (s, bad) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/promote"),
            json!({ "target": "  " }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    assert_eq!(bad["code"], "validation.target");
}

#[tokio::test]
async fn question_rich_fields_round_trip_and_quality_hints() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Prod schema migration").await;
    let fence = app.to_implementing(&id).await;

    // A well-formed choose: options with per-option descriptions, a recommended
    // option, a rationale, confidence, and a summary.
    let (_, body) = app
        .ask(
            &app.worker,
            json!({
            "ticket": id,
            "kind": "choose",
            "title": "How should the migration run?",
            "body": "Long enough body to matter for the summary hint. ".repeat(6),
            "options": [
                { "value": "Run it now", "desc": "Additive & reversible; unblocks today." },
                { "value": "Wait for the window", "desc": "Safer timing, but parks the work ~2 days." }
            ],
            "recommended": "Run it now",
            "recommended_note": "additive and reversible",
            "confidence": 3,
            "summary": "Additive migration — run now or wait for the window.",
            "fence": fence,
        }),
        )
        .await;
    let q = &body["question"];
    assert_eq!(q["options"], json!(["Run it now", "Wait for the window"]));
    assert_eq!(
        q["option_notes"],
        json!([
            "Additive & reversible; unblocks today.",
            "Safer timing, but parks the work ~2 days."
        ])
    );
    assert_eq!(q["confidence"], 3);
    assert_eq!(q["recommended_note"], "additive and reversible");
    assert_eq!(
        q["summary"],
        "Additive migration — run now or wait for the window."
    );
    // Fully specified → no quality hints.
    assert_eq!(
        body["hints"].as_array().map(|a| a.len()),
        Some(0),
        "a complete question should produce no hints: {body}"
    );

    // A bare confirm (no recommendation, long body, no summary) → gets hints.
    let id2 = app.create_ticket("Bump toolchain").await;
    let fence2 = app.to_implementing(&id2).await;
    let (_, body2) = app
        .ask(
            &app.worker,
            json!({
                "ticket": id2,
                "kind": "confirm",
                "title": "OK to bump the toolchain?",
                "body": "A fairly long body that should trigger the summary hint. ".repeat(5),
                "fence": fence2,
            }),
        )
        .await;
    let hints = body2["hints"].as_array().unwrap();
    assert!(
        !hints.is_empty(),
        "a bare question should suggest improvements: {body2}"
    );

    // confidence out of range is refused with a teaching error.
    let id3 = app.create_ticket("Another").await;
    let fence3 = app.to_implementing(&id3).await;
    let (s, bad) = app
        .post(
            &app.worker,
            "/v1/questions",
            json!({ "ticket": id3, "kind": "confirm", "title": "x", "confidence": 5, "fence": fence3 }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    assert_eq!(bad["code"], "validation.confidence");
}

#[tokio::test]
async fn question_choose_validates_options_and_mine_filters_by_expertise() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Which migration strategy?").await;
    let fence = app.to_implementing(&id).await;

    let (qid, _) = app
        .ask(
            &app.worker,
            json!({
                "ticket": id,
                "kind": "choose",
                "title": "Pick a migration strategy",
                "options": ["big-bang", "dual-write", "backfill"],
                "expertise": ["domain:data"],
                "fence": fence,
            }),
        )
        .await;

    // An answer outside the offered options is rejected with a teaching error.
    let (s, bad) = app
        .answer(&app.human, &qid, json!("rewrite-everything"))
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    assert_eq!(bad["code"], "validation.answer");

    // Mint an expert token and confirm ?mine=true routes by expert:<tag> scope.
    let (s, tok) = app
        .post(
            &app.admin,
            "/v1/tokens",
            json!({ "actor": "human:data", "scopes": ["read", "write", "human", "expert:domain:data"] }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{tok}");
    let expert = tok["token"].as_str().unwrap().to_string();

    let (s, mine) = app.get(&expert, "/v1/questions?project=tp&mine=true").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(mine["items"].as_array().unwrap().len(), 1, "{mine}");
    assert_eq!(mine["items"][0]["id"], qid);

    // A billing expert sees nothing under ?mine=true (different tag).
    let (s, tok2) = app
        .post(
            &app.admin,
            "/v1/tokens",
            json!({ "actor": "human:bill", "scopes": ["read", "human", "expert:domain:billing"] }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{tok2}");
    let billing = tok2["token"].as_str().unwrap().to_string();
    let (s, none) = app
        .get(&billing, "/v1/questions?project=tp&mine=true")
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(none["items"].as_array().unwrap().len(), 0, "{none}");

    // The expert answers and resumes the ticket.
    let (s, answered) = app.answer(&expert, &qid, json!("dual-write")).await;
    assert_eq!(s, StatusCode::OK, "{answered}");
    assert_eq!(answered["question"]["answer"]["value"], "dual-write");
    assert_eq!(answered["ticket"]["state"], "ready");
}

/// Asking checks the recommendation against the options, exactly as revising already
/// did (takomo-a0nw). The two paths share `validate_options`, and the recommendation
/// half had drifted out of it: `options: ["a","b"]` with `recommended: "c"` was
/// accepted on the ask path and refused on the revise path.
///
/// It is not cosmetic. `on_timeout=recommended` stores `recommended` as the answer
/// when the deadline passes, so a dangling recommendation is a question that expires
/// into an answer that was never on offer — and the inbox meanwhile renders a
/// recommendation pointing at nothing.
///
/// The rule is scoped to the kinds that carry options: a `confirm` recommends yes/no
/// and a `clarify` a suggested wording, and neither has an option set to be a member
/// of.
#[tokio::test]
async fn ask_validates_the_recommendation_against_the_options() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Recommend something real").await;
    let fence = app.to_implementing(&id).await;

    // The ticket's own repro: a recommendation naming an option that does not exist.
    let (s, e) = app
        .post(
            &app.worker,
            "/v1/questions",
            json!({
                "ticket": id, "kind": "choose", "title": "Which one?",
                "options": ["a", "b"], "recommended": "c", "fence": fence,
            }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert_eq!(e["code"], "validation.recommended");
    let msg = e["message"].as_str().unwrap();
    assert!(msg.contains('c'), "must name the bad recommendation: {e}");
    assert!(msg.contains("a, b"), "must list the real options: {e}");

    // The same input with `on_timeout=recommended` — the combination that would
    // otherwise expire into an answer that was never one of the choices.
    let (s, e) = app
        .post(
            &app.worker,
            "/v1/questions",
            json!({
                "ticket": id, "kind": "choose", "title": "Which one?",
                "options": ["a", "b"], "recommended": "c",
                "expires_in_seconds": 3600, "on_timeout": "recommended", "fence": fence,
            }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert_eq!(e["code"], "validation.recommended");

    // A recommendation of a shape no answer could ever take is refused too: a
    // question with options is answered by naming one of them.
    let (s, e) = app
        .post(
            &app.worker,
            "/v1/questions",
            json!({
                "ticket": id, "kind": "choose", "title": "Which one?",
                "options": ["a", "b"], "recommended": true, "fence": fence,
            }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert_eq!(e["code"], "validation.recommended");

    // recommended_multi is held to the same rule, from the same place.
    let (s, e) = app
        .post(
            &app.worker,
            "/v1/questions",
            json!({
                "ticket": id, "kind": "choose", "title": "Which ones?",
                "options": ["a", "b"], "multi": true,
                "recommended_multi": ["a", "z"], "fence": fence,
            }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert_eq!(e["code"], "validation.recommended_multi");
    assert!(
        e["message"].as_str().unwrap().contains('z'),
        "must name the offending entry: {e}"
    );

    // Kinds that carry no options recommend something free-form, and are untouched:
    // a `confirm` recommends yes/no…
    let (_, ok) = app
        .ask(
            &app.worker,
            json!({
                "ticket": id, "kind": "confirm", "title": "Proceed?",
                "recommended": "yes", "fence": fence,
            }),
        )
        .await;
    assert_eq!(ok["question"]["recommended"], "yes", "{ok}");

    // …and a `clarify` a suggested wording. (Both land on the same ticket, which the
    // blocking barrier allows; only the first parked it.)
    let (_, ok) = app
        .ask(
            &app.worker,
            json!({
                "ticket": id, "kind": "clarify", "title": "Which timezone?",
                "recommended": "store UTC, render local", "fence": fence,
            }),
        )
        .await;
    assert_eq!(
        ok["question"]["recommended"], "store UTC, render local",
        "{ok}"
    );

    // And a recommendation that does name an option is accepted, so the check is a
    // membership test and not a blanket refusal.
    let id2 = app.create_ticket("Recommend a real option").await;
    let fence2 = app.to_implementing(&id2).await;
    let (qid, ok) = app
        .ask(
            &app.worker,
            json!({
                "ticket": id2, "kind": "choose", "title": "Which one?",
                "options": ["a", "b"], "recommended": "b", "fence": fence2,
            }),
        )
        .await;
    assert_eq!(ok["question"]["recommended"], "b", "{ok}");

    // The revise path refuses the identical shape with the identical code — the
    // point of sharing the validator rather than writing a second one.
    let (s, e) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{qid}/options"),
            json!({ "options": ["a", "b"], "recommended": "c" }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert_eq!(e["code"], "validation.recommended");
}

#[tokio::test]
async fn question_withdraw_closes_it_without_answering() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Never mind, found it").await;
    let fence = app.to_implementing(&id).await;
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "clarify", "title": "What does archived mean here?", "fence": fence }),
        )
        .await;

    let (s, w) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{qid}/withdraw"),
            json!({ "reason": "figured it out from the docs" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{w}");
    assert_eq!(w["status"], "withdrawn");

    // It leaves the open inbox.
    let (_, list) = app
        .get(&app.human, "/v1/questions?project=tp&status=open")
        .await;
    assert_eq!(list["items"].as_array().unwrap().len(), 0, "{list}");

    // A withdrawn question can no longer be answered.
    let (s, _) = app.answer(&app.human, &qid, json!("some text")).await;
    assert_eq!(s, StatusCode::CONFLICT);
}

#[tokio::test]
async fn answer_link_lets_an_outsider_answer_once() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Outside review").await;
    let fence = app.to_implementing(&id).await;
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "Ship it?", "fence": fence }),
        )
        .await;

    // A write-only worker cannot mint a link (delegating needs the human scope).
    let (s, _) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{qid}/answer-link"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // A human mints the link.
    let (s, link) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/answer-link"),
            json!({ "actor": "human:contractor" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{link}");
    let token = link["token"].as_str().unwrap().to_string();
    assert!(token.starts_with("tka_"), "grant token: {token}");
    assert!(link["path"].as_str().unwrap().contains("#a="));

    // The outsider (holding ONLY the grant token) sees the one question...
    let (s, self_view) = app.get(&token, "/v1/answer/self").await;
    assert_eq!(s, StatusCode::OK, "{self_view}");
    assert_eq!(self_view["question"]["id"], qid);

    // ...and can answer it, which resumes the ticket.
    let (s, answered) = app
        .post(&token, "/v1/answer/self", json!({ "answer": "yes" }))
        .await;
    assert_eq!(s, StatusCode::OK, "{answered}");
    assert_eq!(answered["ticket"]["state"], "ready");
    assert_eq!(answered["question"]["answered_by"], "human:contractor");

    // The link is single-use: reuse is gone.
    let (s, _) = app.get(&token, "/v1/answer/self").await;
    assert_eq!(s, StatusCode::GONE);
    let (s, _) = app
        .post(&token, "/v1/answer/self", json!({ "answer": "no" }))
        .await;
    assert_eq!(s, StatusCode::GONE);

    // A normal (non-grant) token cannot reach the answer endpoints at all.
    let (s, denied) = app.get(&app.worker, "/v1/answer/self").await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "{denied}");
}

#[tokio::test]
async fn answer_link_delegates_approve_only_with_expertise() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Legal sign-off").await;
    let fence = app.to_implementing(&id).await;
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "approve", "title": "OK legally?", "expertise": ["domain:legal"], "fence": fence }),
        )
        .await;

    // A plain human (no expert:domain:legal) cannot mint a link for an approve
    // question — you can't delegate authority you don't hold.
    let (s, denied) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/answer-link"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{denied}");
    assert_eq!(denied["code"], "question.approve_expertise");

    // A legal expert mints it; the outsider's link then satisfies the approve
    // gate for this one question.
    let (s, tok) = app
        .post(
            &app.admin,
            "/v1/tokens",
            json!({ "actor": "human:counsel", "scopes": ["read", "write", "human", "expert:domain:legal"] }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{tok}");
    let counsel = tok["token"].as_str().unwrap().to_string();
    let (s, link) = app
        .post(
            &counsel,
            &format!("/v1/questions/{qid}/answer-link"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{link}");
    let token = link["token"].as_str().unwrap().to_string();
    let (s, answered) = app
        .post(&token, "/v1/answer/self", json!({ "answer": "yes" }))
        .await;
    assert_eq!(s, StatusCode::OK, "{answered}");
    assert_eq!(answered["ticket"]["state"], "ready");
}

#[tokio::test]
async fn answer_link_revoke_kills_it() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Revoke test").await;
    let fence = app.to_implementing(&id).await;
    let (_, b) = app
        .post(
            &app.worker,
            "/v1/questions",
            json!({ "ticket": id, "kind": "clarify", "title": "Detail?", "fence": fence }),
        )
        .await;
    let qid = b["question"]["id"].as_str().unwrap().to_string();
    let (_, link) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/answer-link"),
            json!({}),
        )
        .await;
    let token = link["token"].as_str().unwrap().to_string();
    let gid = link["id"].as_str().unwrap().to_string();

    // Revoke it, then the token is gone.
    let (s, _) = app
        .delete(&app.human, &format!("/v1/answer-links/{gid}"))
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = app.get(&token, "/v1/answer/self").await;
    assert_eq!(s, StatusCode::GONE);
}

/// The lifetime a grant was actually minted with, read back off the stored row
/// rather than off the response that claimed it — `expires_at - created_at` is
/// what the outside expert's link really honours. Both timestamps come from
/// separate `now_ms()` calls one statement apart, so a millisecond of slack.
async fn mint_answer_link(app: &TestApp, qid: &str, body: Value) -> Value {
    let (s, link) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/answer-link"),
            body,
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{link}");
    link
}

fn granted_ttl_seconds(app: &TestApp, grant_id: &str) -> i64 {
    let row = app
        .open_store()
        .get_answer_grant(grant_id)
        .expect("read grant")
        .unwrap_or_else(|| panic!("grant {grant_id} not stored"));
    let ms = row.expires_at - row.created_at;
    // Round to the nearest second; the two clock reads cannot drift further.
    (ms + 500) / 1000
}

/// A link minted with nothing configured anywhere lives for a week — the
/// captain's ask, and the value an outside expert's link falls back to.
#[tokio::test]
async fn answer_link_defaults_to_one_week() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Default link lifetime").await;
    let fence = app.to_implementing(&id).await;
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "Ship it?", "fence": fence }),
        )
        .await;

    // The seeded project sets no default, so this is the built-in one.
    assert!(
        app.project(&app.worker, "tp").await["answer_link_ttl_seconds"].is_null(),
        "the fixture project must not carry a default, or this asserts nothing"
    );
    let (s, link) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/answer-link"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{link}");
    assert_eq!(link["ttl_seconds"], 604_800, "{link}");
    assert_eq!(link["ttl_source"], "default", "{link}");
    assert_eq!(
        granted_ttl_seconds(&app, link["id"].as_str().unwrap()),
        604_800,
        "the stored grant must expire a week out, not just say so"
    );
}

/// Precedence: an explicit `ttl_seconds` beats the project default, which beats
/// the built-in. All three checked against the stored expiry, so a change that
/// echoes the right number while minting the wrong one still fails.
#[tokio::test]
async fn answer_link_ttl_prefers_explicit_over_project_over_default() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Link lifetime precedence").await;
    let fence = app.to_implementing(&id).await;

    // 1. Nothing set anywhere: the built-in week.
    let (q1, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "One?", "fence": fence }),
        )
        .await;
    let link = mint_answer_link(&app, &q1, json!({})).await;
    assert_eq!(link["ttl_source"], "default");
    assert_eq!(
        granted_ttl_seconds(&app, link["id"].as_str().unwrap()),
        604_800
    );

    // 2. The project sets a default: a link with no explicit ttl takes it.
    let (s, updated) = app
        .put(
            &app.admin,
            "/v1/projects/tp/answer-link-ttl",
            json!({ "ttl_seconds": 86_400 }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{updated}");
    assert_eq!(updated["answer_link_ttl_seconds"], 86_400);
    let link = mint_answer_link(&app, &q1, json!({})).await;
    assert_eq!(link["ttl_source"], "project", "{link}");
    assert_eq!(link["ttl_seconds"], 86_400);
    assert_eq!(
        granted_ttl_seconds(&app, link["id"].as_str().unwrap()),
        86_400,
        "the project default must reach the stored expiry"
    );

    // 3. An explicit ttl on the call still wins over the project default —
    //    whoever looked at this one question knows more than the setting does.
    let link = mint_answer_link(&app, &q1, json!({ "ttl_seconds": 3_600 })).await;
    assert_eq!(link["ttl_source"], "explicit", "{link}");
    assert_eq!(link["ttl_seconds"], 3_600);
    assert_eq!(
        granted_ttl_seconds(&app, link["id"].as_str().unwrap()),
        3_600,
        "an explicit --ttl must not be overwritten by the project default"
    );

    // 4. Clearing the project default falls back to the built-in week again.
    let (s, cleared) = app
        .put(
            &app.admin,
            "/v1/projects/tp/answer-link-ttl",
            json!({ "ttl_seconds": Value::Null }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{cleared}");
    assert!(cleared["answer_link_ttl_seconds"].is_null(), "{cleared}");
    let link = mint_answer_link(&app, &q1, json!({})).await;
    assert_eq!(link["ttl_source"], "default", "{link}");
    assert_eq!(
        granted_ttl_seconds(&app, link["id"].as_str().unwrap()),
        604_800
    );
}

/// The setting is bounded exactly like an explicit `ttl_seconds`, and writing it
/// needs `admin` — a grant that never expires is a standing credential handed to
/// someone outside the org. Reading it does NOT need admin: the /board Settings
/// sheet shows it read-only to any session token.
#[tokio::test]
async fn project_answer_link_ttl_is_bounded_and_admin_only() {
    let app = TestApp::spawn().await;

    // Zero and negative are refused with a teaching 422...
    for bad in [0, -1] {
        let (s, err) = app
            .put(
                &app.admin,
                "/v1/projects/tp/answer-link-ttl",
                json!({ "ttl_seconds": bad }),
            )
            .await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
        assert_eq!(err["code"], "project.answer_link_ttl", "{err}");
    }

    // ...and so is anything past the 30-day cap the share links already carry.
    let (s, err) = app
        .put(
            &app.admin,
            "/v1/projects/tp/answer-link-ttl",
            json!({ "ttl_seconds": 2_592_001 }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
    assert_eq!(err["code"], "project.answer_link_ttl", "{err}");
    assert_eq!(err["details"]["max_seconds"], 2_592_000, "{err}");
    assert!(
        err["message"].as_str().unwrap().contains("2592000"),
        "the message must name the bound it enforces: {err}"
    );
    // Exactly the cap is fine — the bound is inclusive.
    let (s, ok) = app
        .put(
            &app.admin,
            "/v1/projects/tp/answer-link-ttl",
            json!({ "ttl_seconds": 2_592_000 }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{ok}");

    // Omitting the field is an error, not a silent reset to the default.
    let (s, err) = app
        .put(&app.admin, "/v1/projects/tp/answer-link-ttl", json!({}))
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{err}");
    assert_eq!(
        app.project(&app.admin, "tp").await["answer_link_ttl_seconds"],
        2_592_000,
        "a refused write must leave the setting alone"
    );

    // A non-admin can READ the setting — that is what makes the Settings sheet
    // worth opening without admin — but cannot write it.
    assert_eq!(
        app.project(&app.human, "tp").await["answer_link_ttl_seconds"],
        2_592_000,
        "a human token must see the setting to render it read-only"
    );
    for token in [&app.human, &app.worker] {
        let (s, _) = app
            .put(
                token,
                "/v1/projects/tp/answer-link-ttl",
                json!({ "ttl_seconds": 60 }),
            )
            .await;
        assert_eq!(s, StatusCode::FORBIDDEN);
    }
    assert_eq!(
        app.project(&app.admin, "tp").await["answer_link_ttl_seconds"],
        2_592_000,
        "a forbidden write must not have landed"
    );
}

/// Creating a project with an out-of-range default is a clean 422 before the
/// insert, not a project created with the setting quietly dropped.
#[tokio::test]
async fn project_create_rejects_out_of_range_answer_link_ttl() {
    let app = TestApp::spawn().await;
    let (s, err) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({ "id": "ttlp", "name": "TTL", "answer_link_ttl_seconds": 2_592_001 }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
    assert_eq!(err["code"], "project.answer_link_ttl", "{err}");
    let (s, list) = app.get(&app.admin, "/v1/projects").await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        !list.as_array().unwrap().iter().any(|p| p["id"] == "ttlp"),
        "a rejected create must leave nothing behind: {list}"
    );

    // A legal one is stored, and a link minted on that project takes it.
    let (s, made) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({ "id": "ttlp", "name": "TTL", "answer_link_ttl_seconds": 1_209_600 }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    assert_eq!(made["answer_link_ttl_seconds"], 1_209_600);
}

#[tokio::test]
async fn question_recommended_timeout_requires_a_real_window() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("No instant self-approve").await;
    let fence = app.to_implementing(&id).await;

    // on_timeout=recommended with a 1s window is refused: it would let a
    // write-only agent satisfy the human gate almost instantly.
    let (s, body) = app
        .post(
            &app.worker,
            "/v1/questions",
            json!({
                "ticket": id, "kind": "confirm", "title": "Proceed?",
                "recommended": "yes", "expires_in_seconds": 1, "on_timeout": "recommended",
                "fence": fence,
            }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "validation.on_timeout");
}

#[tokio::test]
async fn question_expiry_applies_recommendation() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Auto-resolve on timeout").await;
    let fence = app.to_implementing(&id).await;

    // A valid (>= minimum) recommended-timeout window.
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({
                "ticket": id,
                "kind": "confirm",
                "title": "Proceed if nobody objects?",
                "recommended": "yes",
                "expires_in_seconds": 3600,
                "on_timeout": "recommended",
                "fence": fence,
            }),
        )
        .await;

    // Backdate the deadline directly in the DB (as an aged question would be),
    // so the sweeper picks it up without waiting an hour.
    {
        let conn = rusqlite::Connection::open(app.db_path()).expect("open db");
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let past = takomo::ids::now_ms() - 1000;
        let n = conn
            .execute(
                "UPDATE questions SET expires_at = ?2 WHERE id = ?1",
                rusqlite::params![qid, past],
            )
            .expect("backdate");
        assert_eq!(n, 1);
    }

    // The sweeper runs every 250ms in tests.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let (_, q) = app.get(&app.admin, &format!("/v1/questions/{qid}")).await;
        if q["status"] == "answered" {
            assert_eq!(q["answered_by"], "system");
            assert_eq!(q["answer"]["value"], true);
            let (_, t) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
            assert_eq!(t["state"], "ready", "ticket should resume on timeout");
            break;
        }
        assert!(Instant::now() < deadline, "question was not swept: {q}");
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

/// When a legacy row carries a recommendation that no longer passes
/// `validate_answer`, the timeout fallback must still emit `question_expired`
/// so the event log does not drift from state.
#[tokio::test]
async fn timeout_fallback_emits_question_expired_when_recommendation_is_invalid() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Legacy bad recommendation").await;
    let fence = app.to_implementing(&id).await;

    let (qid, _) = app
        .ask(
            &app.worker,
            json!({
                "ticket": id,
                "kind": "choose",
                "title": "Pick one",
                "options": ["a", "b"],
                "recommended": "a",
                "expires_in_seconds": 3600,
                "on_timeout": "recommended",
                "fence": fence,
            }),
        )
        .await;

    // Simulate a row written before ask-side validation: recommendation no
    // longer matches the option set.
    {
        let conn = rusqlite::Connection::open(app.db_path()).unwrap();
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let past = takomo::ids::now_ms() - 1000;
        let n = conn
            .execute(
                "UPDATE questions SET recommended = ?2, expires_at = ?3 WHERE id = ?1",
                rusqlite::params![qid, "\"c\"", past],
            )
            .unwrap();
        assert_eq!(n, 1);
    }

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let (_, q) = app.get(&app.admin, &format!("/v1/questions/{qid}")).await;
        if q["status"] == "expired" {
            let (_, events) = app
                .get(
                    &app.admin,
                    &format!("/v1/events?since=0&ticket={id}&kind=question_expired"),
                )
                .await;
            assert!(
                events["events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|e| e["payload"]["question"] == qid),
                "question_expired must be logged: {events}"
            );
            break;
        }
        assert!(Instant::now() < deadline, "question was not swept: {q}");
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

#[tokio::test]
async fn project_question_language_surfaces_to_agents() {
    let app = TestApp::spawn().await;

    // Admin sets the project's human-facing question language.
    let (s, body) = app
        .put(
            &app.admin,
            "/v1/projects/tp/language",
            json!({ "language": "German" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body["question_language"], "German");

    // It shows up in the project list…
    let tp = app.project(&app.worker, "tp").await;
    assert_eq!(tp["question_language"], "German");

    // …and the ask response nudges the agent toward that language.
    let id = app.create_ticket("Sprachtest").await;
    let fence = app.to_implementing(&id).await;
    let (_, asked) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "Weitermachen?", "fence": fence }),
        )
        .await;
    assert!(
        asked["note"].as_str().unwrap().contains("German"),
        "ask note should nudge the language: {asked}"
    );

    // Clearing it removes the nudge.
    let (s, cleared) = app
        .put(
            &app.admin,
            "/v1/projects/tp/language",
            json!({ "language": null }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(cleared["question_language"].is_null());

    // Non-admins can't set it.
    let (s, _) = app
        .put(
            &app.worker,
            "/v1/projects/tp/language",
            json!({ "language": "French" }),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

/// PUT /style and /language teach malformed bodies with the shared validation
/// vocabulary — field required, wrong type, unknown field (takomo-xzad).
#[tokio::test]
async fn project_style_and_language_put_validation() {
    let app = TestApp::spawn().await;

    for (path, field) in [
        ("/v1/projects/tp/style", "style_guide"),
        ("/v1/projects/tp/language", "language"),
    ] {
        let (s, missing) = app.put(&app.admin, path, json!({})).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{missing}");
        assert_eq!(missing["code"], "validation.field_required");

        let (s, bad_type) = app.put(&app.admin, path, json!({ field: 42 })).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad_type}");
        assert_eq!(bad_type["code"], "validation.field_type");

        let (s, unknown) = app
            .put(&app.admin, path, json!({ field: "ok", "typo_field": true }))
            .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{unknown}");
        assert_eq!(unknown["code"], "validation.unknown_field");
    }

    // Style guide length is enforced after trim, not on the raw string.
    let padded = format!("{}   ", "x".repeat(2001));
    let (s, err) = app
        .put(
            &app.admin,
            "/v1/projects/tp/style",
            json!({ "style_guide": padded }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
    assert_eq!(err["code"], "project.style_guide_too_long");

    let within_trim = format!("   {}   ", "y".repeat(1990));
    let (s, ok) = app
        .put(
            &app.admin,
            "/v1/projects/tp/style",
            json!({ "style_guide": within_trim }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{ok}");
    assert_eq!(ok["style_guide"].as_str().unwrap().len(), 1990);

    // Project allowlist is a 403 distinct from missing admin scope.
    let (_, minted) = app
        .post(
            &app.admin,
            "/v1/tokens",
            json!({
                "actor": "human:other",
                "scopes": ["read", "write", "human", "admin"],
                "projects": ["demo"]
            }),
        )
        .await;
    let other_proj = minted["token"].as_str().unwrap();
    let (s, denied) = app
        .put(
            other_proj,
            "/v1/projects/tp/style",
            json!({ "style_guide": "nope" }),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{denied}");
    assert_eq!(denied["code"], "auth.project");
}

#[tokio::test]
async fn project_style_guide_surfaces_to_agents() {
    let app = TestApp::spawn().await;
    let guide = "Two sentences max. Plain language, no marketing voice.";

    // Admin sets the project's house style for agent-written text.
    let (s, body) = app
        .put(
            &app.admin,
            "/v1/projects/tp/style",
            json!({ "style_guide": guide }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body["style_guide"], guide);

    // It shows up in the project list…
    let tp = app.project(&app.worker, "tp").await;
    assert_eq!(tp["style_guide"], guide);

    // …and the ask response echoes it back, so an agent that asked before
    // reading the conventions can still fix the question it just wrote.
    let id = app.create_ticket("Style check").await;
    let fence = app.to_implementing(&id).await;
    let (_, asked) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "Proceed?", "fence": fence }),
        )
        .await;
    assert!(
        asked["note"].as_str().unwrap().contains(guide),
        "ask note should carry the style guide: {asked}"
    );

    // A guide over the cap is refused with a teaching 422 — and the previous
    // guide survives, so a bad update never silently wipes the setting.
    let too_long = "x".repeat(2001);
    let (s, err) = app
        .put(
            &app.admin,
            "/v1/projects/tp/style",
            json!({ "style_guide": too_long }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(err["code"], "project.style_guide_too_long");
    assert_eq!(err["details"]["max_chars"], 2000);
    assert_eq!(
        app.project(&app.worker, "tp").await["style_guide"],
        guide,
        "a rejected update must not clear the existing guide"
    );

    // A blank string is a clear, so callers never have to distinguish "" from null.
    let (s, cleared) = app
        .put(
            &app.admin,
            "/v1/projects/tp/style",
            json!({ "style_guide": "   " }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(cleared["style_guide"].is_null(), "{cleared}");

    // Omitting the field is an error, not a silent clear.
    let (s, _) = app
        .put(&app.admin, "/v1/projects/tp/style", json!({}))
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // Non-admins can't set it.
    let (s, _) = app
        .put(
            &app.worker,
            "/v1/projects/tp/style",
            json!({ "style_guide": "anything" }),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn project_create_rejects_oversized_style_guide_before_creating() {
    let app = TestApp::spawn().await;
    let (s, err) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({ "id": "styl", "name": "Style", "style_guide": "x".repeat(2001) }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
    assert_eq!(err["code"], "project.style_guide_too_long");

    // The project must not exist: the guide is validated before the insert, so a
    // rejected create leaves nothing half-configured behind.
    let (_, list) = app.get(&app.admin, "/v1/projects").await;
    assert!(
        !list.as_array().unwrap().iter().any(|p| p["id"] == "styl"),
        "rejected create must not leave a project behind: {list}"
    );

    // A valid one sets the guide at creation time.
    let (s, made) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({ "id": "styl", "name": "Style", "style_guide": "Terse. Imperative mood." }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    assert_eq!(made["style_guide"], "Terse. Imperative mood.");
}

#[tokio::test]
async fn project_create_applies_optional_settings_atomically() {
    let app = TestApp::spawn().await;
    let (s, made) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({
                "id": "atom",
                "name": "Atomic",
                "question_language": "German",
                "style_guide": "Short sentences.",
                "answer_link_ttl_seconds": 604_800,
                "claim_ttl_seconds": 3_600,
                "max_claim_ttl_seconds": 7_200
            }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    assert_eq!(made["question_language"], "German");
    assert_eq!(made["style_guide"], "Short sentences.");
    assert_eq!(made["answer_link_ttl_seconds"], 604_800);
    assert_eq!(made["claim_ttl_seconds"], 3_600);
    assert_eq!(made["max_claim_ttl_seconds"], 7_200);
}

#[tokio::test]
async fn rest_work_loop_carries_project_conventions() {
    let app = TestApp::spawn().await;

    // Nothing set: the work loop must add no keys at all, so a project with no
    // conventions pays no payload for the feature.
    let (s, bare) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "tp", "title": "Before any conventions" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{bare}");
    assert!(bare.get("style_hint").is_none(), "{bare}");
    assert!(bare.get("language_hint").is_none(), "{bare}");

    let guide = "Two sentences max. Plain language, no marketing voice.";
    let (s, _) = app
        .put(
            &app.admin,
            "/v1/projects/tp/style",
            json!({ "style_guide": guide }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = app
        .put(
            &app.admin,
            "/v1/projects/tp/language",
            json!({ "language": "German" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);

    // `takomo new` -> POST /v1/tickets. The hints ride alongside `similar`,
    // which must survive untouched.
    let (s, created) = app
        .post(
            &app.worker,
            "/v1/tickets",
            json!({ "project": "tp", "title": "Styled ticket" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{created}");
    assert_eq!(created["style_hint"]["style_guide"], guide);
    assert_eq!(created["language_hint"]["question_language"], "German");
    assert!(
        created["style_hint"]["note"]
            .as_str()
            .expect("style note")
            .contains("house style"),
        "the hint has to say what to do with it: {created}"
    );
    assert!(created["similar"].is_array(), "{created}");
    let id = created["id"].as_str().expect("ticket id").to_string();

    // `takomo show` -> GET /v1/tickets/{id}.
    let (s, shown) = app.get(&app.worker, &format!("/v1/tickets/{id}")).await;
    assert_eq!(s, StatusCode::OK, "{shown}");
    assert_eq!(shown["style_hint"]["style_guide"], guide);
    assert_eq!(shown["language_hint"]["question_language"], "German");

    // `takomo claim` -> POST /v1/tickets/{id}/claim. The response *is* the lease,
    // so the hints are siblings of its fields; the lease shape must be intact.
    app.to_ready(&id).await;
    let (s, lease) = app
        .post(&app.worker, &format!("/v1/tickets/{id}/claim"), json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{lease}");
    assert_eq!(lease["ticket"], id);
    assert!(lease["fence"].is_i64(), "{lease}");
    assert_eq!(lease["style_hint"]["style_guide"], guide);
    assert_eq!(lease["language_hint"]["question_language"], "German");

    // `takomo next` -> POST /v1/ready/claim, the worker primitive.
    let other = app.create_ticket("Next up").await;
    app.to_ready(&other).await;
    let (s, next) = app
        .post(&app.worker2, "/v1/ready/claim", json!({ "project": "tp" }))
        .await;
    assert_eq!(s, StatusCode::OK, "{next}");
    assert_eq!(next["id"], other);
    assert!(next["lease"]["fence"].is_i64(), "{next}");
    assert_eq!(next["style_hint"]["style_guide"], guide);
    assert_eq!(next["language_hint"]["question_language"], "German");

    // Clearing the guide stops the hint: an agent must never be left following a
    // convention the project has dropped.
    let (s, _) = app
        .put(
            &app.admin,
            "/v1/projects/tp/style",
            json!({ "style_guide": null }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, shown) = app.get(&app.worker, &format!("/v1/tickets/{id}")).await;
    assert!(shown.get("style_hint").is_none(), "{shown}");
    assert_eq!(
        shown["language_hint"]["question_language"], "German",
        "clearing one convention leaves the other: {shown}"
    );
}

#[tokio::test]
async fn ticket_lists_stay_free_of_convention_hints() {
    let app = TestApp::spawn().await;
    let (s, _) = app
        .put(
            &app.admin,
            "/v1/projects/tp/style",
            json!({ "style_guide": "Terse. Imperative mood." }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let id = app.create_ticket("Listed").await;
    app.to_ready(&id).await;

    // The hints are per project, not per ticket: repeating them on every row of a
    // list would mean a project read per row and a payload that grows with the
    // page. List responses stay exactly as they were.
    let (s, list) = app.get(&app.worker, "/v1/tickets?project=tp").await;
    assert_eq!(s, StatusCode::OK, "{list}");
    assert!(list.get("style_hint").is_none(), "{list}");
    for t in list["items"].as_array().expect("items") {
        assert!(t.get("style_hint").is_none(), "{t}");
        assert!(t.get("language_hint").is_none(), "{t}");
    }

    let (s, ready) = app.get(&app.worker, "/v1/ready?project=tp").await;
    assert_eq!(s, StatusCode::OK, "{ready}");
    for t in ready["items"].as_array().expect("ready items") {
        assert!(t.get("style_hint").is_none(), "{t}");
        assert!(t.get("language_hint").is_none(), "{t}");
    }
}

#[tokio::test]
async fn question_barrier_resumes_only_when_all_answered() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Two decisions").await;
    let fence = app.to_implementing(&id).await;

    // Two distinct questions on the same parked ticket.
    let (q1, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "OK to drop the table?", "fence": fence }),
        )
        .await;
    // Second ask: ticket is already parked + unclaimed, so no fence needed.
    let (q2, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "choose", "title": "Which migration?", "options": ["a", "b"] }),
        )
        .await;
    assert_ne!(q1, q2);

    // Answering the first does NOT resume — the barrier is not cleared.
    let (s, a1) = app.answer(&app.human, &q1, json!("yes")).await;
    assert_eq!(s, StatusCode::OK, "{a1}");
    assert!(
        a1["question"]["resolved_to"].is_null(),
        "first answer must not resume: {a1}"
    );
    assert_eq!(a1["ticket"]["state"], "needs-decision");

    // Answering the last one resumes the ticket.
    let (s, a2) = app.answer(&app.human, &q2, json!("a")).await;
    assert_eq!(s, StatusCode::OK, "{a2}");
    assert_eq!(a2["question"]["resolved_to"], "ready");
    assert_eq!(a2["ticket"]["state"], "ready");
}

/// The regression test for takomo-sk4e. `simple` — the workflow `takomo init`
/// applies, and so the one most installs run — has no `scope:human` edge
/// anywhere by design, and the resume lookup used to consider *only*
/// human-gated edges. Every answered blocking question therefore left its
/// ticket parked in `blocked`, out of the ready queue, and said nothing about
/// it. The whole point of ask-a-human is that the answer puts the work back.
#[tokio::test]
async fn question_answer_resumes_on_the_simple_workflow() {
    let app = TestApp::spawn().await;
    app.create_project_with("sw", common::simple_workflow())
        .await;
    let id = app.create_ticket_in("sw", "Needs a call").await;

    // todo -> claim -> in_progress, then park on a blocking question.
    let fence = app.claim(&id).await;
    let (s, b) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/transition"),
            json!({ "to": "in_progress", "fence": fence }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "->in_progress failed: {b}");
    let (qid, asked) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "Drop the column?", "fence": fence }),
        )
        .await;
    assert_eq!(
        asked["ticket"]["state"], "blocked",
        "ask must park: {asked}"
    );

    let (s, ans) = app.answer(&app.human, &qid, json!("yes")).await;
    assert_eq!(s, StatusCode::OK, "answer failed: {ans}");
    // `blocked -> todo` is the only exit that resumes work here: `-> in_progress`
    // is claim-gated and `-> cancelled` is terminal.
    assert_eq!(
        ans["question"]["resolved_to"], "todo",
        "answer must resume the ticket: {ans}"
    );
    assert_eq!(ans["ticket"]["state"], "todo", "{ans}");
    assert_eq!(ans["resume"]["resumed"], true, "{ans}");
    assert_eq!(ans["resume"]["to"], "todo", "{ans}");
    assert!(
        ans["resume"]["code"].is_null(),
        "nothing went wrong, so no reason: {ans}"
    );

    // The point of resuming: the ticket is back in the ready queue, so an agent
    // will actually pick it up again.
    let (_, ready) = app.get(&app.admin, "/v1/ready?project=sw").await;
    let ids: Vec<&str> = ready["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert!(
        ids.contains(&id.as_str()),
        "resumed ticket must be ready: {ready}"
    );
}

/// The widening must not weaken `factory-default`: every exit from
/// `needs-decision` there carries `scope:human`, so the answer still resumes
/// through the approval edge and lands in `ready`, exactly as before.
#[tokio::test]
async fn question_answer_still_resumes_through_the_human_gate() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Gated resume").await;
    let fence = app.to_implementing(&id).await;
    let (qid, asked) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "Ship it?", "fence": fence }),
        )
        .await;
    assert_eq!(asked["ticket"]["state"], "needs-decision");

    let (s, ans) = app.answer(&app.human, &qid, json!("yes")).await;
    assert_eq!(s, StatusCode::OK, "answer failed: {ans}");
    assert_eq!(ans["question"]["resolved_to"], "ready", "{ans}");
    assert_eq!(ans["ticket"]["state"], "ready", "{ans}");
    assert_eq!(ans["resume"]["resumed"], true, "{ans}");
    assert_eq!(ans["resume"]["to"], "ready", "{ans}");
}

/// A workflow whose parked state DOES carry a `scope:human` exit, but where
/// that exit is guarded: the fallback must stay off — a real approval gate is
/// never routed around — and the answer must stop being silent about it. The
/// answer is still recorded (throwing away a human's decision would be worse),
/// but the response carries a machine-readable reason and the ticket thread
/// says the work is stranded.
#[tokio::test]
async fn question_answer_that_cannot_resume_says_so() {
    let app = TestApp::spawn().await;
    app.create_project_with(
        "gated",
        json!({
            "name": "gated",
            "initial": "todo",
            "states": [
                { "id": "todo", "category": "todo", "claimable": true },
                { "id": "doing", "category": "in_progress" },
                { "id": "parked", "category": "blocked" },
                { "id": "done", "category": "done", "terminal": true },
            ],
            "transitions": [
                { "from": "todo", "to": "doing", "requires": ["claim"] },
                { "from": "doing", "to": "parked" },
                { "from": "doing", "to": "done", "requires": ["claim"] },
                // The only exit from `parked` a human could take is guarded, and
                // the other one is terminal.
                { "from": "parked", "to": "todo", "requires": ["scope:human", "guard:no_open_children"] },
                { "from": "parked", "to": "done" },
            ],
            "guards": { "no_open_children": { "description": "every child ticket must be terminal" } },
        }),
    )
    .await;
    let id = app.create_ticket_in("gated", "Stuck either way").await;
    let fence = app.claim(&id).await;
    let (s, b) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/transition"),
            json!({ "to": "doing", "fence": fence }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "->doing failed: {b}");
    let (qid, asked) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "Proceed?", "fence": fence }),
        )
        .await;
    assert_eq!(asked["ticket"]["state"], "parked");

    let (s, ans) = app.answer(&app.human, &qid, json!("yes")).await;
    assert_eq!(s, StatusCode::OK, "the answer is still recorded: {ans}");
    assert_eq!(ans["question"]["status"], "answered", "{ans}");
    assert!(ans["question"]["resolved_to"].is_null(), "{ans}");
    assert_eq!(
        ans["ticket"]["state"], "parked",
        "a guarded human gate is not routed around: {ans}"
    );

    // Not silent: the response says what happened and what to do next.
    assert_eq!(ans["resume"]["resumed"], false, "{ans}");
    assert_eq!(ans["resume"]["code"], "question.no_resume", "{ans}");
    let msg = ans["resume"]["message"].as_str().unwrap_or_default();
    assert!(
        msg.contains("parked"),
        "the reason names the state and its exits: {ans}"
    );
    assert!(
        ans["resume"]["remedy"]
            .as_str()
            .unwrap_or_default()
            .contains("resume_to"),
        "the remedy names the next call: {ans}"
    );

    // And the ticket thread says the work is stranded, for whoever reads the
    // ticket rather than the answer response.
    let (_, detail) = app
        .get(&app.human, &format!("/v1/tickets/{id}?include=comments"))
        .await;
    let bodies: Vec<&str> = detail["comments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["body"].as_str().unwrap())
        .collect();
    assert!(
        bodies.iter().any(|b| b.contains("NOT resumed")),
        "the thread must say the ticket is still parked: {bodies:?}"
    );
}

/// A workflow whose blocked state is `claimable`, so a worker can take a lease
/// on a ticket while it is parked — the one shape where "who may move this
/// ticket" is a live question at answer time.
fn claimable_park_workflow(resume_requires: Value) -> Value {
    json!({
        "name": "leased-park",
        "initial": "todo",
        "states": [
            { "id": "todo", "category": "todo", "claimable": true },
            { "id": "doing", "category": "in_progress" },
            // Claimable *and* blocked: a second worker can hold this ticket
            // while a human decides.
            { "id": "parked", "category": "blocked", "claimable": true },
            { "id": "done", "category": "done", "terminal": true },
        ],
        "transitions": [
            { "from": "todo", "to": "doing", "requires": ["claim"] },
            { "from": "doing", "to": "parked" },
            { "from": "parked", "to": "todo", "requires": resume_requires },
            { "from": "parked", "to": "done", "requires": ["claim"] },
        ],
    })
}

/// Park a ticket in `claimable_park_workflow`'s `parked` state on a blocking
/// question, then take a lease on it as the *second* worker. Returns
/// (ticket id, question id, the second worker's fence).
async fn park_then_lease_as_worker2(app: &TestApp, project: &str) -> (String, String, i64) {
    let id = app.create_ticket_in(project, "Parked but leased").await;
    let fence = app.claim(&id).await;
    let (s, b) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{id}/transition"),
            json!({ "to": "doing", "fence": fence }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "->doing failed: {b}");
    let (qid, asked) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "Proceed?", "fence": fence }),
        )
        .await;
    assert_eq!(asked["ticket"]["state"], "parked", "ask must park: {asked}");
    assert!(
        asked["ticket"]["claim"].is_null(),
        "parking must release the asker's lease: {asked}"
    );

    // A second worker picks the parked ticket up while the human is deciding.
    let fence2 = app.claim_as(&app.worker2, &id).await;
    (id, qid, fence2)
}

/// The resume now runs through `src/store/transition.rs` like any other state
/// change, so the rule that decides *who* may move a claimed ticket applies to
/// it too: only a `scope:human` edge is authoritative over someone else's lease
/// (finding A). Here the workflow declares no human authority on the resume
/// edge, so the answer must not yank the lease — the raw `UPDATE` this replaces
/// moved the ticket and cleared the lease regardless of who held it, leaving the
/// holder writing against a state it never left.
///
/// The decision is still recorded (discarding a human's answer would be worse)
/// and the refusal travels back as `resume_blocked`.
#[tokio::test]
async fn question_answer_does_not_yank_a_live_lease_without_a_human_gate() {
    let app = TestApp::spawn().await;
    app.create_project_with("lp", claimable_park_workflow(json!([])))
        .await;
    let (id, qid, fence2) = park_then_lease_as_worker2(&app, "lp").await;

    let (s, ans) = app.answer(&app.human, &qid, json!("yes")).await;
    assert_eq!(s, StatusCode::OK, "the answer is still recorded: {ans}");
    assert_eq!(ans["question"]["status"], "answered", "{ans}");
    assert!(ans["question"]["resolved_to"].is_null(), "{ans}");
    assert_eq!(
        ans["ticket"]["state"], "parked",
        "the resume must not move a ticket another worker holds: {ans}"
    );
    assert_eq!(
        ans["ticket"]["claim"]["holder"], "agent:w2",
        "the holder's lease must survive the answer: {ans}"
    );
    assert_eq!(ans["resume"]["resumed"], false, "{ans}");
    assert_eq!(
        ans["resume"]["code"], "claim.held",
        "the transition machinery's own refusal is what the caller sees: {ans}"
    );

    // Nothing was written: no `transitioned` event claims the ticket moved.
    let (_, evs) = app
        .get(
            &app.admin,
            &format!("/v1/events?since=0&ticket={id}&kind=transitioned"),
        )
        .await;
    assert!(
        !evs["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["payload"]["to"] == "todo"),
        "the answer must leave no transition behind: {evs}"
    );

    // And the thread says the work is stranded, for whoever reads the ticket.
    let (_, detail) = app
        .get(&app.human, &format!("/v1/tickets/{id}?include=comments"))
        .await;
    let bodies: Vec<&str> = detail["comments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["body"].as_str().unwrap())
        .collect();
    assert!(
        bodies.iter().any(|b| b.contains("NOT resumed")),
        "the thread must say the ticket is still parked: {bodies:?}"
    );

    // Not a dead end: once the holder lets go, the recorded answer's ticket
    // walks the same edge normally.
    let (s, _) = app
        .post(
            &app.worker2,
            &format!("/v1/tickets/{id}/release"),
            json!({ "fence": fence2 }),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, moved) = app.transition(&app.human, &id, "todo").await;
    assert_eq!(s, StatusCode::OK, "->todo failed: {moved}");
    assert_eq!(moved["state"], "todo");
}

/// The mirror image, and why the rule above is the right one rather than merely
/// the strict one: where the workflow *does* mark the resume edge `scope:human`,
/// the answer is authoritative over a lease taken while the ticket was parked.
/// It resumes, releases that lease, and says so in the event log with the same
/// payloads a caller's transition emits — one shape for one logical event.
#[tokio::test]
async fn question_resume_through_the_human_gate_supersedes_a_lease_taken_while_parked() {
    let app = TestApp::spawn().await;
    app.create_project_with("lph", claimable_park_workflow(json!(["scope:human"])))
        .await;
    let (id, qid, _) = park_then_lease_as_worker2(&app, "lph").await;

    let (s, ans) = app.answer(&app.human, &qid, json!("yes")).await;
    assert_eq!(s, StatusCode::OK, "answer failed: {ans}");
    assert_eq!(ans["question"]["resolved_to"], "todo", "{ans}");
    assert_eq!(ans["ticket"]["state"], "todo", "{ans}");
    assert_eq!(ans["resume"]["resumed"], true, "{ans}");
    assert!(
        ans["ticket"]["claim"].is_null(),
        "the human gate supersedes the lease: {ans}"
    );

    // The event log carries the standard pair, attributed to the answerer.
    let (_, trans) = app
        .get(
            &app.admin,
            &format!("/v1/events?since=0&ticket={id}&kind=transitioned"),
        )
        .await;
    let resumed = trans["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["payload"]["to"] == "todo")
        .unwrap_or_else(|| panic!("a transitioned event for the resume: {trans}"))
        .clone();
    assert_eq!(resumed["actor"], "human:reviewer");
    assert_eq!(
        resumed["payload"]["auto_released"], true,
        "the resume released the lease it superseded: {resumed}"
    );
    assert_eq!(
        resumed["payload"]["reason"],
        format!("resolved by human ({qid})")
    );

    let (_, rel) = app
        .get(
            &app.admin,
            &format!("/v1/events?since=0&ticket={id}&kind=released"),
        )
        .await;
    assert!(
        rel["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["payload"]["reason"] == "superseded by human transition"),
        "the release reads the same as any other human-superseded lease: {rel}"
    );
}

#[tokio::test]
async fn question_advisory_on_epic_does_not_park() {
    let app = TestApp::spawn().await;
    // An epic sits in `brief` — which has no self-service park edge, so a
    // blocking question would fail. Advisory works and changes no state.
    let epic = app
        .create_typed("Ship the billing revamp", "epic", None)
        .await;

    let (s, blocked) = app
        .post(
            &app.worker,
            "/v1/questions",
            json!({ "ticket": epic, "kind": "confirm", "title": "Do this epic at all?" }),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "blocking on a brief epic can't park: {blocked}"
    );
    assert_eq!(blocked["code"], "question.no_park");

    let (_, body) = app
        .ask(
            &app.worker,
            json!({ "ticket": epic, "mode": "advisory", "kind": "choose",
                "title": "Which direction for the epic?", "options": ["rewrite", "incremental"],
                "expertise": ["domain:product"] }),
        )
        .await;
    assert_eq!(body["question"]["mode"], "advisory");
    // The epic did not move and holds no claim.
    assert_eq!(body["ticket"]["state"], "brief");
    let qid = body["question"]["id"].as_str().unwrap().to_string();

    // Answering records the decision but changes no ticket state.
    let (s, ans) = app.answer(&app.human, &qid, json!("incremental")).await;
    assert_eq!(s, StatusCode::OK, "{ans}");
    assert_eq!(ans["question"]["status"], "answered");
    assert!(ans["question"]["resolved_to"].is_null());
    assert_eq!(
        ans["ticket"]["state"], "brief",
        "advisory must not move the ticket"
    );
}

#[tokio::test]
async fn question_advisory_does_not_gate_the_barrier() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Blocking + advisory").await;
    let fence = app.to_implementing(&id).await;

    // A blocking question parks the ticket.
    let (blocking, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "OK to proceed?", "fence": fence }),
        )
        .await;
    // An advisory question on the same (now parked, unclaimed) ticket.
    let (advisory, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "mode": "advisory", "kind": "clarify", "title": "FYI: any concerns?" }),
        )
        .await;

    // Answering the advisory one does NOT resume — and, being advisory, never would.
    let (s, _) = app.answer(&app.human, &advisory, json!("none")).await;
    assert_eq!(s, StatusCode::OK);
    let (_, t) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
    assert_eq!(
        t["state"], "needs-decision",
        "advisory answer must not resume"
    );

    // Answering the blocking one resumes, even though... the advisory was the
    // only other open question and advisory never counts toward the barrier.
    let (s, done) = app.answer(&app.human, &blocking, json!("yes")).await;
    assert_eq!(s, StatusCode::OK, "{done}");
    assert_eq!(done["ticket"]["state"], "ready");
}

#[tokio::test]
async fn question_ask_is_idempotent_on_retry() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Retry safe").await;
    let fence = app.to_implementing(&id).await;
    let ask = json!({ "ticket": id, "kind": "confirm", "title": "Same question?", "fence": fence });
    let (s, first) = app.post(&app.worker, "/v1/questions", ask.clone()).await;
    assert_eq!(s, StatusCode::CREATED, "{first}");
    // A retry with identical (asker, kind, title) returns the same question,
    // not a duplicate.
    let (_, again) = app
        .ask(
            &app.worker,
            json!({
                "ticket": id,
                "kind": "confirm",
                "title": "Same question?",
                "mode": "blocking"
            }),
        )
        .await;
    assert_eq!(first["question"]["id"], again["question"]["id"]);
    let (_, list) = app
        .get(&app.human, "/v1/questions?project=tp&status=open")
        .await;
    assert_eq!(
        list["items"].as_array().unwrap().len(),
        1,
        "no duplicate: {list}"
    );
}

#[tokio::test]
async fn question_approve_requires_a_matching_domain_expert() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Approve gate").await;
    let fence = app.to_implementing(&id).await;

    // approve must name an expertise domain.
    let (s, bad) = app
        .post(
            &app.worker,
            "/v1/questions",
            json!({ "ticket": id, "kind": "approve", "title": "Sign off?", "fence": fence }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    assert_eq!(bad["code"], "validation.expertise");

    let (qid, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "approve", "title": "Sign off?", "expertise": ["domain:legal"], "fence": fence }),
        )
        .await;

    // A plain human (no matching expert scope) is refused — approve has teeth.
    let (s, denied) = app.answer(&app.human, &qid, json!("yes")).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{denied}");
    assert_eq!(denied["code"], "question.approve_expertise");

    // The domain expert can.
    let (s, tok) = app
        .post(
            &app.admin,
            "/v1/tokens",
            json!({ "actor": "human:lawyer", "scopes": ["read", "write", "human", "expert:domain:legal"] }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{tok}");
    let expert = tok["token"].as_str().unwrap().to_string();
    let (s, ok) = app.answer(&expert, &qid, json!("yes")).await;
    assert_eq!(s, StatusCode::OK, "{ok}");
    assert_eq!(ok["ticket"]["state"], "ready");
}

// ---------------------------------------------------------------------------
// Security-review hardening (unknown-field rejection, question pagination,
// security headers, malformed-JSON teaching errors).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn token_create_rejects_unknown_field() {
    // A typo'd `expires_seconds` must be a loud 400 — not silently ignored,
    // which would mint a non-expiring token.
    let app = TestApp::spawn().await;
    let (status, body) = app
        .post(
            &app.admin,
            "/v1/tokens",
            json!({ "actor": "agent:x", "scopes": ["read"], "expires_second": 3600 }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "typo'd field must 400: {body}"
    );
    assert_eq!(body["code"], "validation.unknown_field");
}

#[tokio::test]
async fn transition_rejects_unknown_field_so_fence_typo_cannot_slip_through() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("fence typo").await;
    let (status, body) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{id}/transition"),
            json!({ "to": "todo", "fenc": 7 }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "typo'd fence must 400: {body}"
    );
    assert_eq!(body["code"], "validation.unknown_field");
}

#[tokio::test]
async fn questions_list_is_paginated_with_limit_and_cursor() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("q host").await;
    for i in 0..3 {
        app.ask(
            &app.admin,
            json!({
                "ticket": id, "kind": "clarify", "mode": "advisory",
                "title": format!("q{i}"),
            }),
        )
        .await;
    }
    // First page of one, with a cursor for more.
    let (st, page1) = app
        .get(&app.admin, "/v1/questions?status=open&limit=1")
        .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        page1["items"].as_array().unwrap().len(),
        1,
        "page size honored"
    );
    let cursor = page1["next_cursor"].as_i64().expect("next_cursor present");
    // Following the cursor returns the rest.
    let (_, page2) = app
        .get(
            &app.admin,
            &format!("/v1/questions?status=open&limit=1&cursor={cursor}"),
        )
        .await;
    assert_eq!(page2["items"].as_array().unwrap().len(), 1);
    assert_ne!(
        page1["items"][0]["id"], page2["items"][0]["id"],
        "distinct pages"
    );
}

#[tokio::test]
async fn html_apps_send_security_headers() {
    let app = TestApp::spawn().await;
    for path in ["/board", "/inbox"] {
        let resp = app.request(Method::GET, path).send().await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "{path}");
        let h = resp.headers();
        assert!(
            h.get("content-security-policy")
                .and_then(|v| v.to_str().ok())
                .map(|v| v.contains("default-src 'self'"))
                .unwrap_or(false),
            "{path} must send a CSP"
        );
        assert_eq!(
            h.get("x-frame-options").and_then(|v| v.to_str().ok()),
            Some("DENY"),
            "{path}"
        );
        assert_eq!(
            h.get("x-content-type-options")
                .and_then(|v| v.to_str().ok()),
            Some("nosniff"),
            "{path}"
        );
    }
}

#[tokio::test]
async fn malformed_json_body_returns_a_teaching_error() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("bad body").await;
    let resp = app
        .authed(
            Method::POST,
            &app.admin,
            &format!("/v1/tickets/{id}/comments"),
        )
        .header("content-type", "application/json")
        .body("{ this is not json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = resp.json::<Value>().await.unwrap_or(Value::Null);
    assert_eq!(
        body["code"], "validation.json",
        "structured teaching error, got {body}"
    );
}

#[tokio::test]
async fn timeout_recommendation_holds_ticket_while_another_blocking_question_is_open() {
    // The HIGH barrier fix: a recommended-timeout must NOT resume a parked ticket
    // while another open blocking question remains on it.
    let app = TestApp::spawn().await;
    let id = app.create_ticket("two blockers").await;
    let fence = app.to_implementing(&id).await;

    // Q1: recommended-on-timeout.
    let (q1, _) = app
        .ask(
            &app.worker,
            json!({
                "ticket": id, "kind": "confirm", "title": "Q1 auto?",
                "recommended": "yes", "expires_in_seconds": 3600,
                "on_timeout": "recommended", "fence": fence,
            }),
        )
        .await;

    // Q2: a second blocking question on the same (now parked) ticket.
    let (s2, b2) = app
        .post(
            &app.worker,
            "/v1/questions",
            json!({ "ticket": id, "kind": "clarify", "title": "Q2 needs a human" }),
        )
        .await;
    assert_eq!(
        s2,
        StatusCode::CREATED,
        "second blocking ask should be allowed: {b2}"
    );

    // Backdate Q1's deadline so the sweeper fires it.
    {
        let conn = rusqlite::Connection::open(app.db_path()).unwrap();
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        conn.execute(
            "UPDATE questions SET expires_at = ?2 WHERE id = ?1",
            rusqlite::params![q1, takomo::ids::now_ms() - 1000],
        )
        .unwrap();
    }

    // Wait for Q1 to be swept to answered.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let (_, q) = app.get(&app.admin, &format!("/v1/questions/{q1}")).await;
        if q["status"] == "answered" {
            break;
        }
        assert!(Instant::now() < deadline, "Q1 not swept: {q}");
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    // The ticket must still be parked (blocked) because Q2 is open — NOT resumed.
    let (_, t) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
    assert_eq!(
        t["state_category"], "blocked",
        "ticket must stay parked while Q2 is open, got {t}"
    );
}

// ---------------------------------------------------------------------------
// Round-2 review fixes: grant revocation, advisory non-resume, dep scoping.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn answer_link_is_revoked_when_the_question_is_answered_elsewhere() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("grant revoke").await;
    let fence = app.to_implementing(&id).await;
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "Ship?", "fence": fence }),
        )
        .await;
    // Human mints an answer link for an outsider.
    let (s, link) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/answer-link"),
            json!({ "actor": "human:contractor" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{link}");
    let token = link["token"].as_str().unwrap().to_string();
    // An internal human answers the question directly, before the outsider acts.
    let (s, _) = app.answer(&app.human, &qid, json!("yes")).await;
    assert_eq!(s, StatusCode::OK);
    // The outstanding link is now dead — revoked on resolution, not just single-use.
    let (s, _) = app.get(&token, "/v1/answer/self").await;
    assert_ne!(s, StatusCode::OK, "stale link must not resolve");
    let (s2, _) = app
        .post(&token, "/v1/answer/self", json!({ "answer": "no" }))
        .await;
    assert_ne!(s2, StatusCode::OK, "revoked link must not answer");
}

// ---------------------------------------------------------------------------
// Answer-link single-use is a property of ONE transaction (takomo-o4uw): the
// grant is spent in the same tx that records the answer, not by a follow-up
// write. These two tests pin both halves of that — the race, and the row it
// leaves behind.
// ---------------------------------------------------------------------------

/// Eight holders of ONE answer link answer simultaneously. The grant row is the
/// serialization point, so exactly one wins and the other seven are rejected
/// with a message an outside expert can act on — and, crucially, the question
/// carries exactly one answer and one mirrored comment afterwards.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_answers_on_one_link_spend_it_exactly_once() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Outside expert races itself").await;
    let fence = app.to_implementing(&id).await;
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({
                "ticket": id,
                "kind": "choose",
                "title": "Which vendor?",
                "options": ["alpha", "beta"],
                "fence": fence,
            }),
        )
        .await;
    let (s, link) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/answer-link"),
            json!({ "actor": "human:contractor" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{link}");
    let token = link["token"].as_str().unwrap().to_string();
    let gid = link["id"].as_str().unwrap().to_string();

    // Every attempt sends a DIFFERENT answer, so a second write that landed
    // would be visible in the recorded value, not merely in a status code.
    const N: usize = 8;
    let mut futures = Vec::new();
    for i in 0..N {
        let token = token.clone();
        let client = app.client.clone();
        let base = app.base.clone();
        futures.push(tokio::spawn(async move {
            let choice = if i % 2 == 0 { "alpha" } else { "beta" };
            let resp = client
                .post(format!("{base}/v1/answer/self"))
                .bearer_auth(token)
                .json(&json!({ "answer": choice }))
                .send()
                .await
                .expect("answer request");
            let status = resp.status();
            let body = resp.json::<Value>().await.unwrap_or(Value::Null);
            (status, body)
        }));
    }
    let results: Vec<(StatusCode, Value)> = join_all(futures)
        .await
        .into_iter()
        .map(|r| r.expect("join"))
        .collect();

    let winners: Vec<&(StatusCode, Value)> = results
        .iter()
        .filter(|(s, _)| *s == StatusCode::OK)
        .collect();
    assert_eq!(
        winners.len(),
        1,
        "exactly one of {N} concurrent answers may win: {results:?}"
    );

    // EVERY loser is a coherent, teaching 410 — never a 500, never a 409 left
    // over from some other module's bookkeeping, never a silent no-op. A loser
    // is turned away either by the answer-auth path (its link was already spent
    // when the request arrived: `answer.expired`) or, if it got past auth in the
    // window before the winner committed, by the spend inside the answering
    // transaction itself (`answer_link.spent`). Both tell the holder the link
    // was single-use and is gone.
    for (status, body) in results.iter().filter(|(s, _)| *s != StatusCode::OK) {
        assert_eq!(*status, StatusCode::GONE, "loser response: {body}");
        let code = body["code"].as_str().expect("loser code");
        assert!(
            matches!(code, "answer_link.spent" | "answer.expired"),
            "loser needs a stable spent-link code, got: {body}"
        );
        let message = body["message"].as_str().expect("loser message");
        assert!(
            message.contains("single-use"),
            "loser must be told the link was single-use: {message}"
        );
        if code == "answer_link.spent" {
            // The in-transaction rejection also says what became of the question
            // and that nothing of theirs was recorded.
            assert!(
                message.contains("another answer landed first")
                    && message.contains("Nothing was recorded"),
                "in-tx loser must be told their submission did not land: {message}"
            );
            assert!(
                body["remedy"].as_str().is_some_and(|r| !r.is_empty()),
                "in-tx loser needs a remedy: {body}"
            );
        }
    }

    // The state the winner left behind: one answer, one answered event, one
    // mirrored comment. Seven rolled-back transactions wrote nothing.
    let winning_answer = winners[0].1["question"]["answer"]["value"].clone();
    let (s, q) = app.get(&app.human, &format!("/v1/questions/{qid}")).await;
    assert_eq!(s, StatusCode::OK, "{q}");
    assert_eq!(q["status"], "answered");
    assert_eq!(q["answer"]["value"], winning_answer);
    assert_eq!(q["answered_by"], "human:contractor");

    let (_, events) = app
        .get(
            &app.admin,
            &format!("/v1/events?since=0&ticket={id}&kind=question_answered"),
        )
        .await;
    assert_eq!(
        events["events"].as_array().unwrap().len(),
        1,
        "exactly one question_answered event: {events}"
    );
    let (_, ticket) = app
        .get(&app.human, &format!("/v1/tickets/{id}?include=comments"))
        .await;
    let mirrored = ticket["comments"]
        .as_array()
        .expect("comments")
        .iter()
        .filter(|c| {
            c["body"]
                .as_str()
                .is_some_and(|b| b.starts_with(&format!("Human answered {qid}: ")))
        })
        .count();
    assert_eq!(mirrored, 1, "exactly one mirrored answer comment: {ticket}");

    // The grant is spent, not revoked: the winning transaction marked it used,
    // so the resolution sweep (which only touches still-unused links) left it be.
    let grant = app.open_store().get_answer_grant(&gid).unwrap().unwrap();
    assert!(grant.used_at.is_some(), "the winner must spend the link");
    assert!(
        grant.revoked_at.is_none(),
        "a link spent by its own answer is used, not revoked"
    );
}

/// A link the holder spent themselves must report itself USED, not revoked —
/// the distinction the outside expert reads on the 410. It only holds if the
/// spend happens inside the answering transaction, ahead of the sweep that
/// revokes every still-unused link for a resolved question.
#[tokio::test]
async fn a_link_spent_by_its_own_answer_reports_used_not_revoked() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Spent link reports used").await;
    let fence = app.to_implementing(&id).await;
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "Ship it?", "fence": fence }),
        )
        .await;
    let (s, link) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/answer-link"),
            json!({ "actor": "human:contractor" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{link}");
    let token = link["token"].as_str().unwrap().to_string();
    let gid = link["id"].as_str().unwrap().to_string();

    let (s, answered) = app
        .post(&token, "/v1/answer/self", json!({ "answer": "yes" }))
        .await;
    assert_eq!(s, StatusCode::OK, "{answered}");

    let grant = app.open_store().get_answer_grant(&gid).unwrap().unwrap();
    assert!(grant.used_at.is_some(), "used_at must be set by the answer");
    assert!(
        grant.revoked_at.is_none(),
        "the answering link is spent, not revoked: {:?}",
        grant.revoked_at
    );

    // So the holder who reloads their link is told what actually happened.
    for (s, body) in [
        app.get(&token, "/v1/answer/self").await,
        app.post(&token, "/v1/answer/self", json!({ "answer": "no" }))
            .await,
    ] {
        assert_eq!(s, StatusCode::GONE, "{body}");
        assert_eq!(body["code"], "answer.expired", "{body}");
        assert!(
            body["message"]
                .as_str()
                .is_some_and(|m| m.contains("already been used")),
            "a self-spent link must read as used, not revoked: {body}"
        );
    }
}

#[tokio::test]
async fn advisory_question_never_resumes_ticket_on_recommended_timeout() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("advisory no resume").await;
    let fence = app.to_implementing(&id).await;
    // Park the ticket via a blocking question, then withdraw it — the ticket
    // stays blocked with NO open blocking questions.
    let (q1, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "park", "fence": fence }),
        )
        .await;
    let (_, t) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
    assert_eq!(t["state_category"], "blocked", "ticket parked");
    let (s, _) = app
        .post(
            &app.admin,
            &format!("/v1/questions/{q1}/withdraw"),
            json!({ "reason": "n/a" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    // An advisory question with on_timeout=recommended on the still-blocked ticket.
    let (q2, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "mode": "advisory", "title": "adv",
                "recommended": "yes", "expires_in_seconds": 3600, "on_timeout": "recommended" }),
        )
        .await;
    {
        let conn = rusqlite::Connection::open(app.db_path()).unwrap();
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        conn.execute(
            "UPDATE questions SET expires_at = ?2 WHERE id = ?1",
            rusqlite::params![q2, takomo::ids::now_ms() - 1000],
        )
        .unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let (_, q) = app.get(&app.admin, &format!("/v1/questions/{q2}")).await;
        if q["status"] == "answered" {
            break;
        }
        assert!(Instant::now() < deadline, "advisory q not swept: {q}");
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    // Advisory must NEVER touch ticket state — it stays blocked.
    let (_, t) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
    assert_eq!(
        t["state_category"], "blocked",
        "advisory timeout must not resume, got {t}"
    );
}

#[tokio::test]
async fn cross_project_dep_detail_is_hidden_from_a_scoped_token() {
    let app = TestApp::spawn().await;
    let (s, _) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({ "id": "tp2", "name": "Second" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, b) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "tp2", "type": "task", "title": "SECRET infra name" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let secret = b["id"].as_str().unwrap().to_string();
    let a = app.create_ticket("A depends on secret").await;
    let (s, _) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{a}/deps"),
            json!({ "blocked_by": secret }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    // A token scoped ONLY to tp.
    let (s, tk) = app
        .post(
            &app.admin,
            "/v1/tokens",
            json!({ "actor": "agent:scoped", "scopes": ["read", "write"], "projects": ["tp"] }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{tk}");
    let scoped = tk["token"].as_str().unwrap().to_string();
    let (s, detail) = app
        .get(&scoped, &format!("/v1/tickets/{a}?include=deps"))
        .await;
    assert_eq!(s, StatusCode::OK, "{detail}");
    let dep = &detail["deps"]["blocked_by"][0];
    assert_eq!(dep["id"], secret);
    assert_eq!(
        dep["out_of_scope"], true,
        "cross-project dep must be redacted: {dep}"
    );
    assert!(dep.get("title").is_none(), "title must not leak: {dep}");
}

#[tokio::test]
async fn human_can_answer_a_single_choose_with_a_custom_free_text_answer() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("custom answer").await;
    let (qid, _) = app
        .ask(
            &app.admin,
            json!({ "ticket": id, "kind": "choose", "mode": "advisory", "title": "Which path?",
                "options": ["big-bang", "canary"] }),
        )
        .await;
    // A free-text answer that isn't one of the options is rejected WITHOUT the flag.
    let (s, _) = app
        .answer(&app.human, &qid, json!("phased rollout instead"))
        .await;
    assert_eq!(
        s,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a non-option must be rejected normally"
    );
    // With custom:true the human's own instruction is accepted and recorded verbatim.
    let (s, ans) = app
        .answer(
            &app.human,
            &qid,
            json!({ "value": "phased rollout instead", "custom": true }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{ans}");
    let (_, q) = app.get(&app.admin, &format!("/v1/questions/{qid}")).await;
    assert_eq!(q["answer"]["value"], "phased rollout instead");
    assert_eq!(q["answer"]["custom"], true);
}

// ---------------------------------------------------------------------------
// Tag registry + ticket tagging

#[tokio::test]
async fn tag_registry_crud_and_conflict() {
    let app = TestApp::spawn().await;

    // Create a person entity.
    let (s, t) = app
        .post(
            &app.admin,
            "/v1/projects/tp/tags",
            json!({ "kind": "person", "handle": "ada", "label": "Ada Lovelace", "meta": { "email": "ada@x", "role": "eng" } }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    assert_eq!(t["ref"], "person:ada");
    assert_eq!(t["label"], "Ada Lovelace");
    assert_eq!(t["meta"]["email"], "ada@x");

    // Duplicate (project, kind, handle) is a 409.
    let (s, e) = app
        .post(
            &app.admin,
            "/v1/projects/tp/tags",
            json!({ "kind": "person", "handle": "ada" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{e}");
    assert_eq!(e["code"], "tag.exists");

    // A different kind with the same handle is allowed (identity is kind+handle).
    let (s, _) = app
        .post(
            &app.admin,
            "/v1/projects/tp/tags",
            json!({ "kind": "component", "handle": "ada" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);

    // List (ordered by kind, handle) and kind filter.
    let (s, list) = app.get(&app.admin, "/v1/projects/tp/tags").await;
    assert_eq!(s, StatusCode::OK);
    let refs: Vec<&str> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["ref"].as_str().unwrap())
        .collect();
    assert_eq!(
        refs,
        vec!["component:ada", "person:ada"],
        "ordered by kind,handle"
    );
    let (_, people) = app
        .get(&app.admin, "/v1/projects/tp/tags?kind=person")
        .await;
    assert_eq!(people["items"].as_array().unwrap().len(), 1);

    // Substring search on handle/label.
    let (_, q) = app.get(&app.admin, "/v1/projects/tp/tags?q=love").await;
    assert_eq!(q["items"].as_array().unwrap().len(), 1);
    assert_eq!(q["items"][0]["ref"], "person:ada");

    // Get one.
    let (s, one) = app.get(&app.admin, "/v1/projects/tp/tags/person/ada").await;
    assert_eq!(s, StatusCode::OK, "{one}");
    assert_eq!(one["label"], "Ada Lovelace");

    // Patch label + merge meta (null deletes a key).
    let (s, patched) = app
        .patch(
            &app.admin,
            "/v1/projects/tp/tags/person/ada",
            json!({ "label": "Ada L.", "meta_merge": { "role": null, "team": "core" } }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{patched}");
    assert_eq!(patched["label"], "Ada L.");
    assert_eq!(patched["meta"]["email"], "ada@x");
    assert_eq!(patched["meta"]["team"], "core");
    assert!(
        patched["meta"].get("role").is_none(),
        "role deleted via null"
    );

    // Delete; ticket refs (none yet) => still_referenced 0.
    let (s, del) = app
        .delete(&app.admin, "/v1/projects/tp/tags/person/ada")
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(del["still_referenced"], 0);

    // Get after delete is 404.
    let (s, _) = app.get(&app.admin, "/v1/projects/tp/tags/person/ada").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn ticket_tags_lazy_create_patch_and_filter() {
    let app = TestApp::spawn().await;

    // Create a ticket tagged with an unregistered handle: it is lazily registered.
    let (s, t) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "tp", "title": "Invoice rounding", "tags": ["person:ada", "component:billing", "person:ada"] }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let id = t["id"].as_str().unwrap().to_string();
    // Deduped, order preserved.
    assert_eq!(t["tags"], json!(["person:ada", "component:billing"]));

    // The lazy-created registry stub exists (label defaults to the handle).
    let (s, stub) = app.get(&app.admin, "/v1/projects/tp/tags/person/ada").await;
    assert_eq!(s, StatusCode::OK, "{stub}");
    assert_eq!(stub["label"], "ada");

    // tags_add / tags_remove are commutative set ops.
    let (s, patched) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "tags_add": ["team:core"], "tags_remove": ["component:billing"] }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{patched}");
    assert_eq!(patched["tags"], json!(["person:ada", "team:core"]));

    // Whole-set replace.
    let (_, replaced) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "tags": ["person:ada"] }),
        )
        .await;
    assert_eq!(replaced["tags"], json!(["person:ada"]));

    // A second ticket with a different person.
    let (_, t2) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "tp", "title": "Payroll", "tags": ["person:grace"] }),
        )
        .await;
    let id2 = t2["id"].as_str().unwrap().to_string();

    // Filter by exact ref.
    let (_, byref) = app
        .get(&app.admin, "/v1/tickets?project=tp&tag=person:ada")
        .await;
    let ids: Vec<&str> = byref["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert!(
        ids.contains(&id.as_str()) && !ids.contains(&id2.as_str()),
        "tag=person:ada -> {ids:?}"
    );

    // Filter by kind (both tickets carry a person tag).
    let (_, bykind) = app
        .get(&app.admin, "/v1/tickets?project=tp&tag_kind=person")
        .await;
    assert_eq!(
        bykind["items"].as_array().unwrap().len(),
        2,
        "tag_kind=person matches both"
    );

    // Deleting the registry entry reports the tickets still referencing it.
    let (_, del) = app
        .delete(&app.admin, "/v1/projects/tp/tags/person/ada")
        .await;
    assert_eq!(
        del["still_referenced"], 1,
        "one ticket still tags person:ada"
    );
    // The ticket reference survives the registry delete.
    let (_, still) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
    assert_eq!(still["tags"], json!(["person:ada"]));
}

#[tokio::test]
async fn tag_validation_and_scope() {
    let app = TestApp::spawn().await;

    // Malformed ref on a ticket (no colon) is a teaching 422.
    let (s, e) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "tp", "title": "x", "tags": ["ada"] }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert_eq!(e["code"], "validation.tag_ref");

    // Bad kind (uppercase) and bad handle are rejected on registry create.
    let (s, e) = app
        .post(
            &app.admin,
            "/v1/projects/tp/tags",
            json!({ "kind": "Person", "handle": "ada" }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert_eq!(e["code"], "validation.tag_kind");

    let (s, e) = app
        .post(
            &app.admin,
            "/v1/projects/tp/tags",
            json!({ "kind": "person", "handle": "Ada Lovelace" }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert_eq!(e["code"], "validation.tag_handle");

    // Mint a read-only token: it cannot create a tag or tag a ticket.
    let (_, minted) = app
        .post(
            &app.admin,
            "/v1/tokens",
            json!({ "actor": "agent:ro", "scopes": ["read"] }),
        )
        .await;
    let ro = minted["token"].as_str().unwrap().to_string();

    let (s, e) = app
        .post(
            &ro,
            "/v1/projects/tp/tags",
            json!({ "kind": "person", "handle": "ada" }),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{e}");
    assert_eq!(e["code"], "auth.scope");

    let id = app.create_ticket("taggable").await;
    let (s, e) = app
        .patch(
            &ro,
            &format!("/v1/tickets/{id}"),
            json!({ "tags_add": ["person:ada"] }),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{e}");
    assert_eq!(e["code"], "auth.scope");
}

/// Tag registry writes emit audit events; patch guardrails refuse empty and
/// unknown-field bodies (takomo-pbwv).
#[tokio::test]
async fn tag_registry_events_and_patch_guardrails() {
    let app = TestApp::spawn().await;

    let (s, created) = app
        .post(
            &app.admin,
            "/v1/projects/tp/tags",
            json!({ "kind": "team", "handle": "core", "label": "Core" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{created}");

    let (_, ev1) = app
        .get(&app.admin, "/v1/events?since=0&project=tp&kind=tag_created")
        .await;
    assert!(
        ev1["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["payload"]["kind"] == "team" && e["payload"]["handle"] == "core"),
        "tag_created must be logged: {ev1}"
    );

    let (s, patched) = app
        .patch(
            &app.admin,
            "/v1/projects/tp/tags/team/core",
            json!({ "label": "Core platform" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{patched}");

    let (_, ev2) = app
        .get(&app.admin, "/v1/events?since=0&project=tp&kind=tag_updated")
        .await;
    assert!(
        ev2["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["payload"]["kind"] == "team" && e["payload"]["handle"] == "core"),
        "tag_updated must be logged: {ev2}"
    );

    let (s, empty) = app
        .patch(&app.admin, "/v1/projects/tp/tags/team/core", json!({}))
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{empty}");
    assert_eq!(empty["code"], "validation.no_changes");

    let (s, unknown) = app
        .patch(
            &app.admin,
            "/v1/projects/tp/tags/team/core",
            json!({ "label": "Core", "typo": true }),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{unknown}");
    assert_eq!(unknown["code"], "validation.unknown_field");

    let (s, del) = app
        .delete(&app.admin, "/v1/projects/tp/tags/team/core")
        .await;
    assert_eq!(s, StatusCode::OK, "{del}");

    let (_, ev3) = app
        .get(&app.admin, "/v1/events?since=0&project=tp&kind=tag_deleted")
        .await;
    assert!(
        ev3["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["payload"]["kind"] == "team" && e["payload"]["handle"] == "core"),
        "tag_deleted must be logged: {ev3}"
    );

    let (s, nf) = app.get(&app.admin, "/v1/projects/tp/tags/team/core").await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(nf["code"], "notfound.tag");
}

/// A ticket's tag set is bounded (takomo-xrp8). Every reference in a tag write costs
/// a statement inside the single `IMMEDIATE` transaction that holds the process-wide
/// write mutex — the mutex that makes the ready queue hand a ticket to exactly one
/// claimant — so an uncapped `tags` array is a caller-triggered stall of every claim,
/// transition and heartbeat in the store, bought with one request that the per-token
/// write budget counts as one write.
///
/// The cap has to hold on every array that reaches that loop, and on the set a patch
/// would leave behind — a small `tags_add` on an already-full ticket is over the cap
/// too. The refusal names the limit, because a caller that cannot see it can only
/// guess.
#[tokio::test]
async fn ticket_tags_are_capped_per_request_and_per_ticket() {
    let app = TestApp::spawn().await;
    let refs = |n: usize| -> Vec<String> { (0..n).map(|i| format!("component:c{i}")).collect() };

    // Over the cap on create: refused, with the limit and the offending count in
    // `details` so the caller can trim without a second request.
    let (s, e) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "tp", "title": "too many tags", "tags": refs(51) }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert_eq!(e["code"], "tag.too_many");
    assert_eq!(e["details"]["max"], 50, "{e}");
    assert_eq!(e["details"]["count"], 51, "{e}");
    assert_eq!(e["details"]["field"], "tags", "{e}");
    assert!(
        e["message"].as_str().unwrap().contains("50"),
        "the message must name the limit: {e}"
    );
    assert!(e["remedy"].is_string(), "{e}");

    // Exactly at the cap is allowed — the boundary is inclusive — and 50 lazily
    // registered stubs mean exactly 50 `tag_created` events, so the log matches the
    // registry it describes.
    let (s, t) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "tp", "title": "at the cap", "tags": refs(50) }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    assert_eq!(t["tags"].as_array().unwrap().len(), 50, "{t}");
    let id = t["id"].as_str().unwrap().to_string();
    let (_, ev) = app
        .get(&app.admin, "/v1/events?since=0&project=tp&kind=tag_created")
        .await;
    assert_eq!(ev["events"].as_array().unwrap().len(), 50, "{ev}");

    // Re-sending the same set registers nothing new, so it logs nothing new: the
    // lazy-create is one idempotent statement per reference, not a read followed by
    // a write that could double-log.
    let (s, same) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "tags": refs(50) }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{same}");
    let (_, ev2) = app
        .get(&app.admin, "/v1/events?since=0&project=tp&kind=tag_created")
        .await;
    assert_eq!(
        ev2["events"].as_array().unwrap().len(),
        50,
        "an already-registered handle must not emit a second tag_created: {ev2}"
    );

    // A patch may not grow the set past the cap, even with a one-element `tags_add`:
    // what is measured is the set the ticket would end up carrying.
    let (s, e) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "tags_add": ["person:ada"] }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert_eq!(e["code"], "tag.too_many");
    assert_eq!(e["details"]["count"], 51, "{e}");

    // …but swapping one reference for another at the cap is fine, for the same
    // reason: the result is 50 either way.
    let (s, swapped) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "tags_add": ["person:ada"], "tags_remove": ["component:c0"] }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{swapped}");
    assert_eq!(swapped["tags"].as_array().unwrap().len(), 50, "{swapped}");

    // Every array is capped, not just the one that lands. A huge `tags_remove` never
    // reaches the registry loop, but it is still normalized reference by reference
    // inside the write transaction.
    for field in ["tags", "tags_add", "tags_remove"] {
        let mut body = serde_json::Map::new();
        body.insert(field.to_string(), json!(refs(51)));
        let (s, e) = app
            .patch(
                &app.admin,
                &format!("/v1/tickets/{id}"),
                Value::Object(body),
            )
            .await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{field}: {e}");
        assert_eq!(e["code"], "tag.too_many", "{field}: {e}");
        assert_eq!(e["details"]["field"], field, "{field}: {e}");
    }

    // The count is of references sent, before the duplicate drop: bounding the work
    // means bounding the array the write path walks, not only its distinct entries.
    let (s, e) = app
        .patch(
            &app.admin,
            &format!("/v1/tickets/{id}"),
            json!({ "tags": vec!["person:ada"; 51] }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert_eq!(e["code"], "tag.too_many", "{e}");
}

/// A project's lease policy is configurable, and the numbers it sets are the ones
/// claims actually get (takomo-2ztv).
///
/// Both halves matter and fail independently: a setting the claim path ignores is
/// worse than no setting, because the board would show a policy the fleet is not
/// following.
#[tokio::test]
async fn project_claim_ttl_settings_drive_real_leases() {
    let app = TestApp::spawn().await;

    // Unset: the built-ins. Pinned so a change to either constant has to be a
    // deliberate edit here rather than a silent shift in every deployment.
    let p = app.project(&app.admin, "tp").await;
    assert!(p["claim_ttl_seconds"].is_null(), "{p}");
    assert!(p["max_claim_ttl_seconds"].is_null(), "{p}");
    let id = app.create_ticket("lease policy").await;
    app.transition(&app.admin, &id, "spec").await;
    let (s, lease) = app
        .post(&app.worker, &format!("/v1/tickets/{id}/claim"), json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{lease}");
    assert_lease_seconds(&lease, 900, "nothing configured falls back to the built-in");

    // Raise both. The ceiling is above the built-in 3600 on purpose — that a
    // project can exceed it is the point of the ticket.
    let (s, updated) = app
        .put(
            &app.admin,
            "/v1/projects/tp/claim-ttl",
            json!({ "ttl_seconds": 7_200, "max_ttl_seconds": 21_600 }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{updated}");
    assert_eq!(updated["claim_ttl_seconds"], 7_200);
    assert_eq!(updated["max_claim_ttl_seconds"], 21_600);

    // The project's default now reaches a real lease…
    let other = app.create_ticket("lease policy 2").await;
    app.transition(&app.admin, &other, "spec").await;
    let (_, lease) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{other}/claim"),
            json!({}),
        )
        .await;
    assert_lease_seconds(
        &lease,
        7_200,
        "a claim naming no ttl_seconds takes the project default",
    );

    // …and an explicit request above the OLD built-in maximum is now allowed,
    // which is the whole point: 4 hours would have been a 422 before this.
    let third = app.create_ticket("lease policy 3").await;
    app.transition(&app.admin, &third, "spec").await;
    let (s, lease) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{third}/claim"),
            json!({ "ttl_seconds": 14_400 }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{lease}");
    assert_lease_seconds(
        &lease,
        14_400,
        "4h is allowed once the project raises its ceiling",
    );

    // Over the project's own ceiling is still a refusal, and it teaches with the
    // project's numbers rather than the built-in ones.
    let (s, err) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{third}/heartbeat"),
            json!({ "fence": lease["fence"], "ttl_seconds": 21_601 }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
    assert_eq!(err["code"], "validation.ttl", "{err}");
    assert!(
        err["message"]
            .as_str()
            .unwrap_or_default()
            .contains("21600"),
        "the refusal must name the project's ceiling, not the built-in 3600: {err}"
    );

    // Cleared, both fall back to the built-ins again — not to the last value set.
    let (s, cleared) = app
        .put(
            &app.admin,
            "/v1/projects/tp/claim-ttl",
            json!({ "ttl_seconds": null, "max_ttl_seconds": null }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{cleared}");
    assert!(cleared["claim_ttl_seconds"].is_null(), "{cleared}");
    let fourth = app.create_ticket("lease policy 4").await;
    app.transition(&app.admin, &fourth, "spec").await;
    let (_, lease) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{fourth}/claim"),
            json!({}),
        )
        .await;
    assert_lease_seconds(
        &lease,
        900,
        "cleared settings fall back to the built-in, not the last value",
    );
}

/// The lease pair is validated together, admin-only, and refuses the combination
/// that would silently do nothing.
#[tokio::test]
async fn project_claim_ttl_is_validated_as_a_pair_and_admin_only() {
    let app = TestApp::spawn().await;

    // A default above the ceiling that would apply is the trap worth catching: it
    // would be clamped on every claim, so the project would be configured to a
    // number no claim ever gets.
    let (s, err) = app
        .put(
            &app.admin,
            "/v1/projects/tp/claim-ttl",
            json!({ "ttl_seconds": 7_200 }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
    assert_eq!(err["code"], "project.claim_ttl", "{err}");
    assert_eq!(
        err["details"]["effective_max_seconds"], 3_600,
        "the error must name the ceiling that would actually apply: {err}"
    );
    // …and the same pair in one request is fine, which is why they share an
    // endpoint.
    let (s, ok) = app
        .put(
            &app.admin,
            "/v1/projects/tp/claim-ttl",
            json!({ "ttl_seconds": 7_200, "max_ttl_seconds": 7_200 }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{ok}");

    // Omitting a field leaves it alone rather than clearing it — this endpoint
    // writes both columns, so "absent" must not mean NULL.
    let (s, kept) = app
        .put(
            &app.admin,
            "/v1/projects/tp/claim-ttl",
            json!({ "ttl_seconds": 1_800 }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{kept}");
    assert_eq!(kept["claim_ttl_seconds"], 1_800);
    assert_eq!(
        kept["max_claim_ttl_seconds"], 7_200,
        "an omitted max must be preserved, not reset to the built-in: {kept}"
    );

    // Neither field is a 400, so a typo'd name cannot silently reset the policy.
    let (s, err) = app
        .put(&app.admin, "/v1/projects/tp/claim-ttl", json!({}))
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{err}");

    // Non-positive is refused.
    let (s, err) = app
        .put(
            &app.admin,
            "/v1/projects/tp/claim-ttl",
            json!({ "ttl_seconds": 0 }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
    assert_eq!(err["code"], "project.claim_ttl", "{err}");

    // Lease policy decides how long work can be held, so it is admin, not write.
    let (s, err) = app
        .put(
            &app.worker,
            "/v1/projects/tp/claim-ttl",
            json!({ "ttl_seconds": 600 }),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{err}");

    // No upper bound on the ceiling — the captain's explicit call in takomo-2ztv.
    // Asserted rather than left implicit, because the obvious "safety" patch is to
    // add one, and that would silently break a deployment relying on this.
    let (s, big) = app
        .put(
            &app.admin,
            "/v1/projects/tp/claim-ttl",
            json!({ "ttl_seconds": null, "max_ttl_seconds": 604_800 }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "a week-long ceiling is allowed: {big}");
    assert_eq!(big["max_claim_ttl_seconds"], 604_800);
}

/// Assert a lease is good for about `want` seconds, from its `expires_at`.
///
/// Tolerant by one second in each direction, deliberately: the lease was granted
/// a few milliseconds before this reads the clock, so the remaining time is just
/// under the configured whole number and which way it truncates depends on where
/// in the current second the request landed. A tolerance is the difference between
/// pinning the setting and pinning the scheduler.
fn assert_lease_seconds(lease: &Value, want: i64, what: &str) {
    let expires = chrono::DateTime::parse_from_rfc3339(
        lease["expires_at"].as_str().expect("lease has expires_at"),
    )
    .expect("expires_at is RFC3339")
    .timestamp();
    let got = expires - chrono::Utc::now().timestamp();
    assert!(
        (got - want).abs() <= 1,
        "{what}: lease should run ~{want}s but runs {got}s ({lease})"
    );
}

/// An agent can write down a decision a human already made, and cannot
/// manufacture one (takomo-22xj).
///
/// The `human` gate on answering is about *authority*. What it was accidentally
/// also blocking is *transcription*: a human reads a parked question, decides
/// out of band, and the orchestrating agent could not record it — so the
/// question stayed open and the ticket stayed blocked over bookkeeping. The
/// relay path fixes that without handing an agent the authority itself, and this
/// pins both halves, because either one alone is worthless.
#[tokio::test]
async fn an_agent_may_relay_a_human_decision_but_never_invent_one() {
    let app = TestApp::spawn().await;
    // read,write + the relay scope, and deliberately NOT `human`.
    let relay = app.mint(
        "agent:orchestrator",
        &["read", "write", "answer:relay"],
        None,
    );

    let id = app.create_ticket("relay a decision").await;
    let fence = app.to_implementing(&id).await;
    let (q1, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "Ship it?", "fence": fence }),
        )
        .await;

    // Without the scope at all, nothing changes: still the human gate.
    let (s, err) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{q1}/answer"),
            json!({ "answer": "yes", "on_behalf_of": "human:christian" }),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{err}");
    assert_eq!(err["code"], "auth.scope", "{err}");

    // With it, the answer lands — attributed to the human who decided, not to
    // the agent that typed it.
    let (s, ok) = app
        .post(
            &app.worker,
            &format!("/v1/questions/{q1}/answer"),
            json!({ "answer": "yes" }),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "a worker still cannot answer: {ok}"
    );

    let (s, ok) = app
        .post(
            &relay,
            &format!("/v1/questions/{q1}/answer"),
            json!({ "answer": "yes", "on_behalf_of": "human:christian" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "relay should be accepted: {ok}");
    assert_eq!(
        ok["question"]["answered_by"], "human:christian",
        "the DECIDER owns the answer, not the relayer — this field is what a \
         later reader holds someone to: {ok}"
    );
    // The ticket must actually MOVE. This is the point of the feature, and
    // getting it wrong is worse than not shipping it: a parked ticket is out of
    // the ready queue, so an answer that resolves the question without resuming
    // the ticket leaves work that no fleet picks up and no inbox shows — the
    // question now reads as answered. Silently stalled beats nothing, and
    // visibly blocked beats both.
    assert!(
        ok["resume"]["resumed"].as_bool().unwrap_or(false),
        "a relayed answer must resume the ticket, not just close the question — \
         otherwise it strands work outside the ready queue with nothing surfacing \
         it: {ok}"
    );
    let (_, fresh) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
    assert_ne!(
        fresh["state_category"], "blocked",
        "the ticket must be out of the blocked category after a relayed answer: {fresh}"
    );

    // …and the log says how it got there, so a relayed decision is
    // distinguishable from a first-hand one.
    let (_, evs) = app
        .get(
            &app.admin,
            "/v1/events?since=0&limit=200&kind=question_answered",
        )
        .await;
    let ev = evs["events"]
        .as_array()
        .expect("events")
        .iter()
        .find(|e| e["payload"]["question"] == json!(q1))
        .expect("the answer event");
    assert_eq!(
        ev["actor"], "human:christian",
        "the event's actor is who decided: {ev}"
    );
    assert_eq!(
        ev["payload"]["relayed_by"], "agent:orchestrator",
        "and the payload says who transcribed it: {ev}"
    );

    // The invariant that makes the scope safe to hand out: you cannot relay a
    // question you asked yourself. Otherwise `answer:relay` is merely "an agent
    // may answer" — ask for approval, grant it to yourself, resume the ticket.
    // The relay token has to be the ASKER here, so it must hold the lease
    // itself — claim and start as that actor rather than reusing the worker's
    // fence, or the ask is refused for the wrong reason and the test proves
    // nothing about relaying.
    let id2 = app.create_ticket("self-relay must fail").await;
    app.to_ready(&id2).await;
    let (s, lease) = app
        .post(&relay, &format!("/v1/tickets/{id2}/claim"), json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "relay token claims: {lease}");
    let fence2 = lease["fence"].as_i64().expect("fence");
    let (s, b) = app
        .post(
            &relay,
            &format!("/v1/tickets/{id2}/transition"),
            json!({ "to": "implementing", "fence": fence2 }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "->implementing: {b}");
    let (qid2, _) = app
        .ask(
            &relay,
            json!({ "ticket": id2, "kind": "confirm", "title": "May I?", "fence": fence2 }),
        )
        .await;
    let (s, err) = app
        .post(
            &relay,
            &format!("/v1/questions/{qid2}/answer"),
            json!({ "answer": "yes", "on_behalf_of": "human:christian" }),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{err}");
    assert_eq!(err["code"], "answer.relay_self", "{err}");
    assert!(
        err["remedy"].as_str().unwrap_or_default().contains("human"),
        "the refusal must name the way out: {err}"
    );

    // A token that CAN answer should answer, not claim to be relaying — else
    // `answered_by` becomes a claim instead of a fact.
    let id3 = app.create_ticket("human relaying is redundant").await;
    let fence3 = app.to_implementing(&id3).await;
    let (q3, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id3, "kind": "confirm", "title": "Fine?", "fence": fence3 }),
        )
        .await;
    let (s, err) = app
        .post(
            &app.human,
            &format!("/v1/questions/{q3}/answer"),
            json!({ "answer": "yes", "on_behalf_of": "human:someone-else" }),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{err}");
    assert_eq!(err["code"], "answer.relay_redundant", "{err}");
}

/// An `approve` question is never relayable — its whole point is proof that a
/// named expert exercised their authority, and a relayed name is a claim about
/// who decided rather than evidence of it (takomo-22xj).
#[tokio::test]
async fn approve_questions_cannot_be_relayed_even_with_the_scope() {
    let app = TestApp::spawn().await;
    let relay = app.mint(
        "agent:orchestrator",
        &["read", "write", "answer:relay"],
        None,
    );
    // Even a relay token that ALSO holds the expertise must not shortcut it:
    // holding the scope is what answering an approve is supposed to demonstrate.
    let relay_expert = app.mint(
        "agent:orchestrator2",
        &["read", "write", "answer:relay", "expert:domain:billing"],
        None,
    );

    let id = app.create_ticket("approve is not relayable").await;
    let fence = app.to_implementing(&id).await;
    let (q, _) = app
        .ask(
            &app.worker,
            json!({
                "ticket": id,
                "kind": "approve",
                "title": "Re-price 1,800 live subscriptions?",
                "expertise": ["domain:billing"],
                "fence": fence,
            }),
        )
        .await;

    for (token, who) in [
        (&relay, "a plain relay token"),
        (&relay_expert, "a relay token holding the expertise"),
    ] {
        let (s, err) = app
            .post(
                token,
                &format!("/v1/questions/{q}/answer"),
                json!({ "answer": "yes", "on_behalf_of": "human:christian" }),
            )
            .await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{who} must be refused: {err}");
        assert_eq!(err["code"], "answer.relay_approve", "{who}: {err}");
    }

    // The direct expert path is untouched.
    let expert = app.mint(
        "human:cfo",
        &["read", "write", "human", "expert:domain:billing"],
        None,
    );
    let (s, ok) = app
        .post(
            &expert,
            &format!("/v1/questions/{q}/answer"),
            json!({ "answer": "yes" }),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::OK,
        "the expert can still approve directly: {ok}"
    );
    assert_eq!(ok["question"]["answered_by"], "human:cfo", "{ok}");
}

// ---------------------------------------------------------------------------
// Initiatives — the read surface. Initiatives are written over MCP (see
// tests/mcp.rs); these tests build fixtures straight in the store and then drive
// the four REST routes a UI reads them through.

/// Create an initiative with a text entry and an attachment entry, returning
/// (initiative id, text entry id, attachment entry id).
fn seed_initiative(store: &takomo::store::Store) -> (String, String, String) {
    let ini = store
        .create_initiative(
            "tp",
            &takomo::store::InitiativeCreate {
                title: "Name the thing".to_string(),
                summary: Some("Every project needs a good name.".to_string()),
                labels: vec!["naming".to_string()],
                tags: vec!["person:ada".to_string()],
                ..Default::default()
            },
            "human:admin",
        )
        .expect("create initiative");
    let (note, _) = store
        .append_initiative_entry(
            &ini.id,
            &takomo::store::EntryCreate {
                kind: "note".to_string(),
                text: "Shortlist: takomo, kombu, nori.".to_string(),
                source: "claude:chat".to_string(),
                ..Default::default()
            },
            "agent:w1",
        )
        .expect("append note");
    let (doc, _) = store
        .append_initiative_entry(
            &ini.id,
            &takomo::store::EntryCreate {
                kind: "document".to_string(),
                text: "Trademark search results.".to_string(),
                content: Some(b"%PDF-1.7 fake".to_vec()),
                mime: Some("application/pdf".to_string()),
                filename: Some("trademarks.pdf".to_string()),
                source: "person:ada".to_string(),
                ..Default::default()
            },
            "human:reviewer",
        )
        .expect("append document");
    (ini.id, note.id, doc.id)
}

/// The rollup is the feature: a caller must be able to see how much has piled up
/// on an initiative without reading any of it. It is derived, never stored, so
/// these numbers have to add up to exactly what the entries hold.
#[tokio::test]
async fn initiative_list_and_detail_report_a_derived_rollup() {
    let app = TestApp::spawn().await;
    let (id, _, _) = seed_initiative(&app.open_store());

    let (status, body) = app.get(&app.worker, "/v1/initiatives?project=tp").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    let listed = &body["items"][0];
    assert_eq!(listed["id"], id);
    assert_eq!(listed["title"], "Name the thing");
    assert_eq!(listed["status"], "open");
    assert_eq!(listed["tags"][0], "person:ada");

    // Two entries, one of them an attachment.
    let rollup = &listed["rollup"];
    assert_eq!(rollup["entries"], 2);
    assert_eq!(rollup["attachments"], 1);
    let text_chars = "Shortlist: takomo, kombu, nori.".chars().count() as i64
        + "Trademark search results.".chars().count() as i64;
    assert_eq!(rollup["chars"], text_chars);
    assert_eq!(rollup["attachment_bytes"], 13);
    assert_eq!(rollup["bytes"], text_chars + 13);
    assert!(
        rollup["last_entry_at"].is_string(),
        "an initiative with entries reports when the last one landed: {rollup}"
    );

    // The detail route agrees with the list — same derivation, one code path.
    let (status, detail) = app.get(&app.worker, &format!("/v1/initiatives/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["rollup"], *rollup);
}

/// The rollup's two ATTENTION counts, against every rule that decides whether a
/// document is waiting on someone.
///
/// These exist so a collection can be read without opening each document in turn,
/// which means they are only worth having if they agree with what the page shows.
/// `web/src/lib/initiative-doc.ts` decides that with three predicates — superseded,
/// resolved, decided — plus a pane filter, and each one is exercised here with a
/// row that must NOT count next to a row that must. A count that ignored any of
/// them would still look plausible on a healthy document and would nag forever on
/// a finished one.
#[tokio::test]
async fn initiative_rollup_counts_what_is_waiting_on_a_person() {
    let app = TestApp::spawn().await;
    let (id, note_id, _) = seed_initiative(&app.open_store());
    let entries = format!("/v1/initiatives/{id}/entries");

    let append = async |body: serde_json::Value| -> String {
        let (status, out) = app.post(&app.human, &entries, body).await;
        assert_eq!(status, StatusCode::CREATED, "{out}");
        out["entry"]["id"].as_str().unwrap().to_string()
    };
    let rollup = async || -> serde_json::Value {
        let (status, d) = app.get(&app.worker, &format!("/v1/initiatives/{id}")).await;
        assert_eq!(status, StatusCode::OK);
        d["rollup"].clone()
    };

    // Nothing is waiting on an initiative that is only evidence.
    let r = rollup().await;
    assert_eq!(r["open_notes"], 0);
    assert_eq!(r["pending_amendments"], 0);

    // The live pane, which is not an amendment however many times it is revised.
    append(json!({
        "kind": "view", "source": "agent:w1", "text": "The live position.",
        "meta": { "pane": "business", "cites": [] },
    }))
    .await;

    // One note, one undecided amendment: both are waiting.
    let open_note = append(json!({
        "kind": "thread", "source": "person:ada", "text": "Who counted?",
        "meta": { "pane": "business", "para": 0 },
    }))
    .await;
    let undecided = append(json!({
        "kind": "view", "source": "agent:w3", "text": "A better position.",
        "meta": { "pane": "business", "cites": [], "proposed": true },
    }))
    .await;
    let r = rollup().await;
    assert_eq!(r["open_notes"], 1, "{r}");
    assert_eq!(r["pending_amendments"], 1, "{r}");

    // A note explicitly resolved stops waiting.
    append(json!({
        "kind": "thread", "source": "person:ada", "text": "Answered inline.",
        "meta": { "pane": "business", "para": 0, "state": "resolved" },
    }))
    .await;
    assert_eq!(
        rollup().await["open_notes"],
        1,
        "a resolved note is not waiting"
    );

    // `running` is not resolved — work is under way, the question still stands.
    append(json!({
        "kind": "thread", "source": "agent:w1", "text": "Counting now.",
        "meta": { "pane": "business", "para": 0, "state": "running" },
    }))
    .await;
    assert_eq!(rollup().await["open_notes"], 2, "'running' still counts");

    // Superseding a note replaces it rather than adding to it: the superseding
    // row counts, the superseded one stops.
    append(json!({
        "kind": "thread", "source": "agent:w1", "text": "Filed as work.",
        "meta": { "pane": "business", "para": 0, "supersedes": open_note, "state": "running" },
    }))
    .await;
    assert_eq!(
        rollup().await["open_notes"],
        2,
        "a superseding note replaces the one it supersedes"
    );

    // A decision retires the amendment whether it accepts or rejects it.
    append(json!({
        "kind": "decision", "source": "human:reviewer", "text": "No.",
        "meta": { "rejects": undecided, "pane": "business" },
    }))
    .await;
    assert_eq!(
        rollup().await["pending_amendments"],
        0,
        "a rejected amendment is decided"
    );

    let accepted = append(json!({
        "kind": "view", "source": "agent:w3", "text": "Another try.",
        "meta": { "pane": "technical", "cites": [], "proposed": true },
    }))
    .await;
    assert_eq!(rollup().await["pending_amendments"], 1);
    append(json!({
        "kind": "decision", "source": "human:reviewer", "text": "Yes.",
        "meta": { "accepts": accepted, "pane": "technical" },
    }))
    .await;
    assert_eq!(rollup().await["pending_amendments"], 0);

    // Document machinery pointed at no real pane is invisible on the page, so it
    // must be invisible here too — otherwise the badge nags about something the
    // reader can never find.
    append(json!({
        "kind": "thread", "source": "agent:w9", "text": "Stray.",
        "meta": { "pane": "nonsense", "para": 0 },
    }))
    .await;
    append(json!({
        "kind": "view", "source": "agent:w9", "text": "Stray.",
        "meta": { "pane": "nonsense", "cites": [], "proposed": true },
    }))
    .await;
    // …and neither is an entry with no `meta` at all: evidence is not a note.
    append(json!({
        "kind": "research", "source": "somewhere", "text": "Evidence.",
    }))
    .await;
    let r = rollup().await;
    assert_eq!(r["open_notes"], 2, "{r}");
    assert_eq!(r["pending_amendments"], 0, "{r}");

    // The list agrees with the detail route — one derivation, as with every other
    // rollup field.
    let (status, body) = app.get(&app.worker, "/v1/initiatives?project=tp").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"][0]["rollup"], r);
    let _ = note_id;
}

/// Entries carry their provenance, and listing them never drags attachment bytes
/// along — `has_content` is how a reader learns there are any.
#[tokio::test]
async fn initiative_entries_list_carries_provenance_but_not_bytes() {
    let app = TestApp::spawn().await;
    let (id, note_id, doc_id) = seed_initiative(&app.open_store());

    let (status, body) = app
        .get(&app.worker, &format!("/v1/initiatives/{id}/entries"))
        .await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    // Newest first.
    assert_eq!(items[0]["id"], doc_id);
    assert_eq!(items[1]["id"], note_id);

    let doc = &items[0];
    assert_eq!(doc["kind"], "document");
    assert_eq!(doc["source"], "person:ada");
    assert_eq!(doc["author"], "human:reviewer");
    assert_eq!(doc["has_content"], true);
    assert_eq!(doc["content_bytes"], 13);
    assert_eq!(doc["filename"], "trademarks.pdf");
    assert!(
        doc.get("content").is_none(),
        "the entry list must never carry attachment bytes: {doc}"
    );

    let note = &items[1];
    assert_eq!(note["source"], "claude:chat");
    assert_eq!(note["has_content"], false);
    assert_eq!(note["content_bytes"], 0);

    // The collection's rollup rides along, so a UI rendering the entry list does
    // not need a second request to head its page.
    assert_eq!(body["rollup"]["entries"], 2);
}

/// `meta` survives the round-trip verbatim, on both the append response and the
/// entry list.
///
/// This is the contract the /initiatives DOCUMENT view is built on: three `view`
/// entries carry one pane's prose each, `meta.cites` maps a `[n]` mark in that
/// prose to the entry it came from, and `thread` entries anchor a margin note to
/// a paragraph. All of it rides in a column the store has always had — so the
/// document needed no schema change and no new route, and this test is what
/// says the field cannot be quietly dropped from the response.
#[tokio::test]
async fn initiative_entry_meta_round_trips_for_the_document_view() {
    let app = TestApp::spawn().await;
    let (id, note_id, _) = seed_initiative(&app.open_store());

    let (status, appended) = app
        .post(
            &app.worker,
            &format!("/v1/initiatives/{id}/entries"),
            json!({
                "kind": "view",
                "text": "The problem, stated once[1].",
                "source": "agent:w1",
                "meta": { "pane": "business", "cites": [note_id] },
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{appended}");
    assert_eq!(appended["entry"]["kind"], "view");
    assert_eq!(appended["entry"]["meta"]["pane"], "business");
    assert_eq!(appended["entry"]["meta"]["cites"][0], note_id);

    let (status, thread) = app
        .post(
            &app.worker,
            &format!("/v1/initiatives/{id}/entries"),
            json!({
                "kind": "thread",
                "text": "Who counted?",
                "source": "person:ada",
                "meta": { "pane": "business", "para": 0, "state": "open" },
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{thread}");

    let (status, body) = app
        .get(&app.worker, &format!("/v1/initiatives/{id}/entries"))
        .await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().unwrap();

    let view = items
        .iter()
        .find(|e| e["kind"] == "view")
        .expect("the view entry is listed");
    assert_eq!(view["meta"]["pane"], "business");
    assert_eq!(view["meta"]["cites"][0], note_id);

    let margin = items
        .iter()
        .find(|e| e["kind"] == "thread")
        .expect("the thread entry is listed");
    assert_eq!(margin["meta"]["para"], 0);
    assert_eq!(margin["meta"]["state"], "open");

    // An entry appended without `meta` reports an empty object rather than null,
    // so a reader can index into it without a guard.
    let plain = items
        .iter()
        .find(|e| e["id"] == note_id)
        .expect("the seeded note is listed");
    assert_eq!(plain["meta"], json!({}), "{plain}");
}

/// The amendment loop, end to end over HTTP: propose, then decide.
///
/// Every step is an APPEND. A proposed pane carries `meta.proposed`, which keeps
/// it out of the live document until a person acts; accepting appends the prose
/// as a real `view` PLUS a `decision` naming the proposal; rejecting appends the
/// decision alone and leaves the pane untouched. Nothing is edited or deleted, so
/// the wording that lost and the person who decided are both still readable.
///
/// No route exists for any of this — it is the entry surface all the way down,
/// which is the property this test is really pinning.
#[tokio::test]
async fn initiative_amendments_and_dispatch_are_appends() {
    let app = TestApp::spawn().await;
    let (id, note_id, _) = seed_initiative(&app.open_store());
    let entries = format!("/v1/initiatives/{id}/entries");

    // The live pane.
    let (status, _) = app
        .post(
            &app.human,
            &entries,
            json!({
                "kind": "view", "source": "agent:w1", "text": "The live position.",
                "meta": { "pane": "business", "cites": [] },
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    // An amendment, which must NOT become the live pane on its own.
    let (status, proposed) = app
        .post(
            &app.human,
            &entries,
            json!({
                "kind": "view", "source": "agent:w3", "text": "The revised position[1].",
                "meta": { "pane": "business", "cites": [note_id], "proposed": true },
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let proposal_id = proposed["entry"]["id"].as_str().unwrap().to_string();
    assert_eq!(proposed["entry"]["meta"]["proposed"], true);

    // Accepting: the prose as a real view, then the decision.
    for body in [
        json!({
            "kind": "view", "source": "human:reviewer", "text": "The revised position[1].",
            "meta": { "pane": "business", "cites": [note_id], "from": proposal_id },
        }),
        json!({
            "kind": "decision", "source": "human:reviewer", "text": "Accepted.",
            "meta": { "accepts": proposal_id, "pane": "business" },
        }),
    ] {
        let (status, out) = app.post(&app.human, &entries, body).await;
        assert_eq!(status, StatusCode::CREATED, "{out}");
    }

    // A margin note, then the dispatch that supersedes it as `running`.
    let (_, note) = app
        .post(
            &app.human,
            &entries,
            json!({
                "kind": "thread", "source": "person:ada", "text": "Who counted?",
                "meta": { "pane": "business", "para": 1 },
            }),
        )
        .await;
    let thread_id = note["entry"]["id"].as_str().unwrap().to_string();

    let (status, ticket) = app
        .post(
            &app.human,
            "/v1/tickets",
            json!({ "project": "tp", "title": "Count multi-site accounts", "tags": [format!("initiative:{id}")] }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{ticket}");
    let ticket_id = ticket["id"].as_str().unwrap().to_string();

    let (status, superseding) = app
        .post(
            &app.human,
            &entries,
            json!({
                "kind": "thread", "source": "human:reviewer", "text": "Who counted?",
                "meta": {
                    "pane": "business", "para": 1, "state": "running",
                    "ticket": ticket_id, "supersedes": thread_id,
                },
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{superseding}");

    // Every step is still there: two views, the proposal, the decision, and BOTH
    // threads. The reducer picks the winners; the store forgets nothing.
    let (status, body) = app.get(&app.human, &entries).await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().unwrap();
    let kinds: Vec<&str> = items.iter().filter_map(|e| e["kind"].as_str()).collect();
    assert_eq!(kinds.iter().filter(|k| **k == "view").count(), 3);
    assert_eq!(kinds.iter().filter(|k| **k == "thread").count(), 2);
    assert_eq!(kinds.iter().filter(|k| **k == "decision").count(), 1);

    let decision = items.iter().find(|e| e["kind"] == "decision").unwrap();
    assert_eq!(decision["meta"]["accepts"], proposal_id);
    let dispatched = items
        .iter()
        .find(|e| e["meta"]["state"] == "running")
        .expect("the superseding thread is listed");
    assert_eq!(dispatched["meta"]["ticket"], ticket_id);
    assert_eq!(dispatched["meta"]["supersedes"], thread_id);
}

/// The attachment route is the only non-JSON endpoint in the API. It must serve
/// the stored bytes under the entry's own media type, as a download rather than
/// something a browser will render.
#[tokio::test]
async fn initiative_attachment_downloads_with_its_own_type() {
    let app = TestApp::spawn().await;
    let (id, note_id, doc_id) = seed_initiative(&app.open_store());

    let resp = app
        .authed(
            Method::GET,
            &app.worker,
            &format!("/v1/initiatives/{id}/entries/{doc_id}/content"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let headers = resp.headers().clone();
    assert_eq!(headers["content-type"], "application/pdf");
    assert_eq!(
        headers["content-disposition"], "attachment; filename=\"trademarks.pdf\"",
        "agent-supplied bytes are served as a download, never inline"
    );
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert_eq!(resp.bytes().await.unwrap().as_ref(), b"%PDF-1.7 fake");

    // A text-only entry has no bytes to fetch, and says so rather than serving an
    // empty body that reads like a corrupt download.
    let (status, body) = app
        .get(
            &app.worker,
            &format!("/v1/initiatives/{id}/entries/{note_id}/content"),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("text-only")
            || body["remedy"]
                .as_str()
                .unwrap_or_default()
                .contains("text-only"),
        "the refusal should say the entry is text-only: {body}"
    );
}

/// An entry id from one initiative must not be readable through another's path.
/// The pair is the address, not the entry id alone.
#[tokio::test]
async fn initiative_entry_is_scoped_to_its_initiative() {
    let app = TestApp::spawn().await;
    let store = app.open_store();
    let (_, _, doc_id) = seed_initiative(&store);
    let other = store
        .create_initiative(
            "tp",
            &takomo::store::InitiativeCreate {
                title: "Something else".to_string(),
                ..Default::default()
            },
            "human:admin",
        )
        .unwrap();

    let (status, _) = app
        .get(
            &app.worker,
            &format!("/v1/initiatives/{}/entries/{doc_id}/content", other.id),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "an entry must not be reachable through a different initiative's path"
    );
}

/// A token restricted to other projects cannot read an initiative just by naming
/// its id — the project check runs against the initiative's own project.
#[tokio::test]
async fn initiative_reads_respect_the_project_allowlist() {
    let app = TestApp::spawn().await;
    let (id, _, doc_id) = seed_initiative(&app.open_store());
    let outsider = app.mint("agent:elsewhere", &["read", "write"], Some(&["other"]));

    for path in [
        format!("/v1/initiatives/{id}"),
        format!("/v1/initiatives/{id}/entries"),
        format!("/v1/initiatives/{id}/entries/{doc_id}/content"),
    ] {
        let (status, _) = app.get(&outsider, &path).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{path} must be refused to a token scoped to another project"
        );
    }

    // And the list simply does not include it.
    let (status, body) = app.get(&outsider, "/v1/initiatives").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["items"].as_array().unwrap().len(),
        0,
        "the list must be bounded by the token's project allowlist: {body}"
    );
}

/// Filters and paging, since a UI depends on both.
#[tokio::test]
async fn initiative_list_filters_and_pages() {
    let app = TestApp::spawn().await;
    let store = app.open_store();
    for (title, status, label) in [
        ("Naming the product", "open", "naming"),
        ("Pricing experiments", "parked", "pricing"),
        ("Onboarding rewrite", "open", "onboarding"),
    ] {
        store
            .create_initiative(
                "tp",
                &takomo::store::InitiativeCreate {
                    title: title.to_string(),
                    status: Some(status.to_string()),
                    labels: vec![label.to_string()],
                    ..Default::default()
                },
                "human:admin",
            )
            .unwrap();
    }

    let (_, body) = app.get(&app.worker, "/v1/initiatives?status=open").await;
    assert_eq!(body["items"].as_array().unwrap().len(), 2);

    let (_, body) = app.get(&app.worker, "/v1/initiatives?label=pricing").await;
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    assert_eq!(body["items"][0]["title"], "Pricing experiments");

    // Tokenized search: every term must match, against title or summary.
    let (_, body) = app
        .get(&app.worker, "/v1/initiatives?q=naming%20product")
        .await;
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    let (_, body) = app
        .get(&app.worker, "/v1/initiatives?q=naming%20pricing")
        .await;
    assert_eq!(
        body["items"].as_array().unwrap().len(),
        0,
        "terms are ANDed, not ORed: {body}"
    );

    // Paging walks the whole set exactly once.
    let (_, page1) = app.get(&app.worker, "/v1/initiatives?limit=2").await;
    assert_eq!(page1["items"].as_array().unwrap().len(), 2);
    let cursor = page1["next_cursor"].as_str().expect("a second page exists");
    let (_, page2) = app
        .get(
            &app.worker,
            &format!("/v1/initiatives?limit=2&cursor={cursor}"),
        )
        .await;
    assert_eq!(page2["items"].as_array().unwrap().len(), 1);
    assert!(page2["next_cursor"].is_null());
}

/// An initiative that does not exist is a 404, not an empty document — and a
/// garbage cursor is a teaching 400 rather than a silently empty page.
#[tokio::test]
async fn initiative_reads_refuse_bad_input_clearly() {
    let app = TestApp::spawn().await;
    let (id, _, _) = seed_initiative(&app.open_store());

    let (status, body) = app.get(&app.worker, "/v1/initiatives/ini-nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "notfound.initiative");

    let (status, body) = app
        .get(
            &app.worker,
            &format!("/v1/initiatives/{id}/entries?cursor=abc"),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "validation.cursor");
}

// ---------------------------------------------------------------------------
// Initiative WRITE routes. They exist because /initiatives — the page — needs
// them: an initiative is fed by people as well as agents, and a browser cannot
// call an MCP tool. Every one goes through the same store method as the matching
// MCP tool, so these tests are about the HTTP shape and the refusals.

/// The loop the page drives: create, retitle, park, append an entry with an
/// attachment, and watch the rollup follow.
#[tokio::test]
async fn initiative_write_routes_drive_the_page_loop() {
    let app = TestApp::spawn().await;

    let (status, ini) = app
        .post(
            &app.human,
            "/v1/initiatives",
            json!({
                "project": "tp",
                "title": "Name the thing",
                "summary": "Every project needs a good name.",
                "labels": ["naming"],
                "tags": ["person:ada"],
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = ini["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("ini-"), "unexpected id shape: {id}");
    assert_eq!(ini["status"], "open");
    assert_eq!(ini["labels"][0], "naming");
    assert_eq!(ini["tags"][0], "person:ada");
    assert_eq!(ini["rollup"]["entries"], 0);

    // Retitle and park, the two edits the detail pane makes in place.
    let (status, updated) = app
        .patch(
            &app.human,
            &format!("/v1/initiatives/{id}"),
            json!({ "title": "Naming the product", "status": "parked" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["title"], "Naming the product");
    assert_eq!(updated["status"], "parked");
    assert_eq!(updated["version"], 2);

    // Append an entry with an attachment, the composer's main job.
    let (status, appended) = app
        .post(
            &app.human,
            &format!("/v1/initiatives/{id}/entries"),
            json!({
                "kind": "feedback",
                "source": "person:ada",
                "title": "Ada's note",
                "text": "Prefer something pronounceable.",
                "source_uri": "https://example.test/thread/9",
                "origin_at": "2026-07-01T09:00:00Z",
                // "hello takomo"
                "content_base64": "aGVsbG8gdGFrb21v",
                "mime": "text/plain",
                "filename": "note.txt",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(appended["entry"]["kind"], "feedback");
    assert_eq!(appended["entry"]["source"], "person:ada");
    assert_eq!(appended["entry"]["author"], "human:reviewer");
    assert_eq!(appended["entry"]["origin_at"], "2026-07-01T09:00:00.000Z");
    assert_eq!(appended["entry"]["has_content"], true);
    assert_eq!(appended["entry"]["content_bytes"], 12);
    // The refreshed initiative rides along so the page can update its rollup
    // without a second request.
    assert_eq!(appended["initiative"]["rollup"]["entries"], 1);
    assert_eq!(appended["initiative"]["rollup"]["attachments"], 1);

    // A parked initiative is still appendable — parking is not closing.
    let (status, _) = app
        .post(
            &app.human,
            &format!("/v1/initiatives/{id}/entries"),
            json!({ "kind": "note", "source": "person:ada", "text": "One more thought." }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    // And the bytes come back through the content route.
    let entry_id = appended["entry"]["id"].as_str().unwrap();
    let resp = app
        .authed(
            Method::GET,
            &app.human,
            &format!("/v1/initiatives/{id}/entries/{entry_id}/content"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.bytes().await.unwrap().as_ref(), b"hello takomo");
}

/// Deleting an initiative: what goes, what stays, and what refuses.
///
/// The three kinds of reference an initiative can carry are each handled the way
/// the code that created them already decided — so this test is really three
/// assertions about consistency with decisions made elsewhere in the store.
#[tokio::test]
async fn deleting_an_initiative_takes_its_entries_and_leaves_the_work() {
    let app = TestApp::spawn().await;
    let (id, _note, doc) = seed_initiative(&app.open_store());

    // An epic filed under the lane, by the tag the roadmap grows lanes from.
    let (s, epic) = app
        .post(
            &app.human,
            "/v1/tickets",
            json!({
                "project": "tp",
                "type": "epic",
                "title": "v1",
                "tags": [format!("initiative:{id}")],
            }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{epic}");
    let epic_id = epic["id"].as_str().unwrap().to_string();

    // The lane is real on the roadmap before the delete.
    let (s, rm) = app.get(&app.human, "/v1/projects/tp/roadmap").await;
    assert_eq!(s, StatusCode::OK, "{rm}");
    assert!(
        rm["initiatives"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["id"] == json!(id)),
        "lane should be on the roadmap first: {rm}"
    );

    let (s, gone) = app
        .delete(&app.human, &format!("/v1/initiatives/{id}"))
        .await;
    assert_eq!(s, StatusCode::OK, "{gone}");
    assert_eq!(gone["id"], json!(id));
    assert_eq!(gone["entries"], 2, "both entries went with it: {gone}");
    assert!(gone["bytes"].as_i64().unwrap() > 0, "{gone}");
    assert_eq!(gone["detached_checks"], 0);
    // Counted, so the caller knows a lane is about to vanish from the roadmap.
    assert_eq!(gone["tagged_tickets"], 1, "{gone}");

    // The initiative and its entries are unreachable…
    let (s, _) = app.get(&app.human, &format!("/v1/initiatives/{id}")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let resp = app
        .authed(
            Method::GET,
            &app.human,
            &format!("/v1/initiatives/{id}/entries/{doc}/content"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // …and the WORK it named is untouched, tag and all. A ticket whose
    // `initiative:` tag names nothing stays visible under `uninitiated` — the
    // roadmap's own rule for a dangling reference, which this must not break.
    let (s, ticket) = app.get(&app.human, &format!("/v1/tickets/{epic_id}")).await;
    assert_eq!(s, StatusCode::OK, "{ticket}");
    assert_eq!(ticket["tags"][0], json!(format!("initiative:{id}")));

    let (s, rm) = app.get(&app.human, "/v1/projects/tp/roadmap").await;
    assert_eq!(s, StatusCode::OK, "{rm}");
    assert!(
        !rm["initiatives"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["id"] == json!(id)),
        "the lane is gone: {rm}"
    );
    assert!(
        rm["epics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["id"] == json!(epic_id)),
        "the epic is still reported: {rm}"
    );
}

/// A check's `initiative` is validated on write, so a delete may not leave one
/// naming nothing. It refuses and names them; forced, it detaches them rather
/// than orphaning them — `NULL` is a value that column already has a meaning for.
#[tokio::test]
async fn deleting_an_initiative_refuses_while_a_check_still_names_it() {
    let app = TestApp::spawn().await;
    let (s, ini) = app
        .post(
            &app.human,
            "/v1/initiatives",
            json!({ "project": "tp", "title": "Billing" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{ini}");
    let id = ini["id"].as_str().unwrap().to_string();

    let (s, check) = app
        .post(
            &app.admin,
            "/v1/projects/tp/checks",
            json!({ "title": "Invoice totals", "initiative": id }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{check}");
    let check_id = check["id"].as_str().unwrap().to_string();

    let (s, refused) = app
        .delete(&app.human, &format!("/v1/initiatives/{id}"))
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["code"], "conflict.initiative_has_checks");
    assert_eq!(refused["details"]["checks"][0], json!(check_id));
    // Refused means nothing happened.
    let (s, _) = app.get(&app.human, &format!("/v1/initiatives/{id}")).await;
    assert_eq!(s, StatusCode::OK);

    let (s, gone) = app
        .delete(&app.human, &format!("/v1/initiatives/{id}?force=true"))
        .await;
    assert_eq!(s, StatusCode::OK, "{gone}");
    assert_eq!(gone["detached_checks"], 1, "{gone}");

    // The check survived, detached rather than dangling — and the store still
    // agrees, which a dangling id would have made impossible.
    let (s, after) = app.get(&app.admin, &format!("/v1/checks/{check_id}")).await;
    assert_eq!(s, StatusCode::OK, "{after}");
    assert!(after["initiative"].is_null(), "{after}");
}

/// Deleting is a `write`, scoped to the initiative's own project — naming an id
/// must never reach one the token may not see, the rule `patch` already follows.
#[tokio::test]
async fn deleting_an_initiative_needs_scope_and_respects_projects() {
    let app = TestApp::spawn().await;
    let (s, ini) = app
        .post(
            &app.human,
            "/v1/initiatives",
            json!({ "project": "tp", "title": "Mine" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{ini}");
    let id = ini["id"].as_str().unwrap().to_string();

    let reader = app.mint("human:reader", &["read"], None);
    let (s, denied) = app.delete(&reader, &format!("/v1/initiatives/{id}")).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{denied}");

    let elsewhere = app.mint("agent:elsewhere", &["read", "write"], Some(&["op"]));
    let (s, denied) = app
        .delete(&elsewhere, &format!("/v1/initiatives/{id}"))
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{denied}");

    let (s, missing) = app.delete(&app.human, "/v1/initiatives/ini-nosuchid").await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{missing}");
    assert_eq!(missing["code"], "notfound.initiative");

    // Still there after every refusal.
    let (s, _) = app.get(&app.human, &format!("/v1/initiatives/{id}")).await;
    assert_eq!(s, StatusCode::OK);
}

/// The refusals the page can walk into, each with the code the UI branches on.
#[tokio::test]
async fn initiative_write_routes_refuse_bad_input() {
    let app = TestApp::spawn().await;
    let (_, ini) = app
        .post(
            &app.human,
            "/v1/initiatives",
            json!({ "project": "tp", "title": "Name the thing" }),
        )
        .await;
    let id = ini["id"].as_str().unwrap().to_string();

    // A typo'd field is refused, never silently dropped — the repo-wide norm.
    let (status, body) = app
        .post(
            &app.human,
            "/v1/initiatives",
            json!({ "project": "tp", "title": "x", "sumary": "typo" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "validation.unknown_field");

    // A patch with nothing in it would bump the version for no change.
    let (status, body) = app
        .patch(&app.human, &format!("/v1/initiatives/{id}"), json!({}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "validation.no_changes");

    // Status is a fixed vocabulary even though it is only a label.
    let (status, body) = app
        .patch(
            &app.human,
            &format!("/v1/initiatives/{id}"),
            json!({ "status": "shipped" }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["code"], "validation.initiative_status");

    // The entry refusals the composer has to render.
    for (code, expect, body_json) in [
        (
            "validation.entry_source",
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "kind": "note", "source": " ", "text": "x" }),
        ),
        (
            "validation.entry_empty",
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "kind": "note", "source": "person:ada" }),
        ),
        (
            "validation.entry_content_base64",
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "kind": "document", "source": "p", "content_base64": "no!", "mime": "text/plain" }),
        ),
        (
            "validation.entry_attachment_unlabeled",
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "kind": "document", "source": "p", "content_base64": "aGk=" }),
        ),
        (
            "validation.origin_at",
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "kind": "note", "source": "p", "text": "x", "origin_at": "last tuesday" }),
        ),
        (
            "validation.field_required",
            StatusCode::BAD_REQUEST,
            json!({ "source": "p", "text": "x" }),
        ),
    ] {
        let (status, body) = app
            .post(
                &app.human,
                &format!("/v1/initiatives/{id}/entries"),
                body_json.clone(),
            )
            .await;
        assert_eq!(status, expect, "{code} for {body_json}: {body}");
        assert_eq!(body["code"], code, "wrong code for {body_json}: {body}");
    }

    // An unknown initiative is a 404 on both write paths.
    for (method_is_patch, path) in [
        (true, "/v1/initiatives/ini-nope".to_string()),
        (false, "/v1/initiatives/ini-nope/entries".to_string()),
    ] {
        let (status, body) = if method_is_patch {
            app.patch(&app.human, &path, json!({ "status": "parked" }))
                .await
        } else {
            app.post(
                &app.human,
                &path,
                json!({ "kind": "note", "source": "p", "text": "x" }),
            )
            .await
        };
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body}");
        assert_eq!(body["code"], "notfound.initiative");
    }
}

/// Writing needs the `write` scope, and the project allowlist bounds it — the
/// page's "this token can only read" state depends on the first, and naming an id
/// must never be a way past the second.
#[tokio::test]
async fn initiative_writes_need_scope_and_respect_projects() {
    let app = TestApp::spawn().await;
    let (_, ini) = app
        .post(
            &app.human,
            "/v1/initiatives",
            json!({ "project": "tp", "title": "Name the thing" }),
        )
        .await;
    let id = ini["id"].as_str().unwrap().to_string();
    let reader = app.mint("agent:reader", &["read"], None);
    let outsider = app.mint("agent:elsewhere", &["read", "write"], Some(&["other"]));

    for token in [&reader, &outsider] {
        let (status, _) = app
            .post(
                token,
                "/v1/initiatives",
                json!({ "project": "tp", "title": "Sneaky" }),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = app
            .patch(
                token,
                &format!("/v1/initiatives/{id}"),
                json!({ "status": "parked" }),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = app
            .post(
                token,
                &format!("/v1/initiatives/{id}/entries"),
                json!({ "kind": "note", "source": "p", "text": "x" }),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
}

/// The /initiatives page is served, carries the shared markdown renderer, and
/// links the other two surfaces. Asserted against the bytes the binary serves,
/// because the pages are `include_str!`'d and a stale build is the classic way to
/// be wrong about them.
#[tokio::test]
async fn initiatives_page_is_served_with_the_shared_renderer() {
    let app = TestApp::spawn().await;
    let resp = app
        .request(Method::GET, "/initiatives")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    // Same defense-in-depth headers as every page route: these hold a bearer
    // token in localStorage.
    let headers = resp.headers().clone();
    assert_eq!(headers["x-frame-options"], "DENY");
    assert!(headers["content-security-policy"]
        .to_str()
        .unwrap()
        .contains("frame-ancestors 'none'"));
    assert_app_shell("/initiatives", &resp.text().await.unwrap());

    // The renderer itself is a bundled module, so asserting on its source text
    // would only assert on minifier output. Its behaviour is covered properly by
    // 30 tests in web/src/lib/markdown.test.ts, including the scheme allowlist
    // and the markup-injection cases nothing verified before the port. What this
    // layer can still prove is that the initiatives vocabulary ships at all.
    let bundle = app.eager_bundle().await;
    assert!(
        bundle.contains("md-table"),
        "the markdown renderer is missing"
    );

    // `/initiatives` is an eagerly imported route, so its client ships on first
    // load. Which FILE it lands in is the bundler's business: `lib/initiatives`
    // also holds `whoami` and `listProjects`, which every other surface imports,
    // so it gets hoisted into whatever shared chunk the graph implies that
    // build — it had a chunk of its own once and is folded into `app.js` now,
    // from identical source. Asserting the filename tested the bundler; assert
    // the property instead.
    assert!(
        app.eager_bundle().await.contains("/v1/initiatives"),
        "the initiatives client is missing from first load, so the page cannot \
         fetch anything"
    );
}

// ---------------------------------------------------------------------------
// Environments: where a check can be run.
//
// The registry exists because a verdict with no environment behind it is a claim
// nobody can reproduce, and because the URL, the bring-up command and the
// credential pointer were previously carried out of band — which is exactly the
// context that goes stale without anyone noticing.
//
// What is worth pinning here is not the CRUD but the three rules a later change
// is most likely to erode: production is read-only unless someone says
// otherwise, a slug is immutable, and this table never holds a secret.
// ---------------------------------------------------------------------------

/// Register an environment and return the created body.
async fn environment(app: &TestApp, body: Value) -> Value {
    let (status, b) = app
        .post(&app.admin, "/v1/projects/tp/environments", body)
        .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "environment create failed: {b}"
    );
    b
}

/// The defaults are the documentation an agent actually reads, so they are the
/// part worth asserting: an undescribed environment says `unknown` rather than
/// guessing, and production is read-only until somebody states otherwise.
#[tokio::test]
async fn an_environment_defaults_to_unknown_data_and_production_to_read_only() {
    let app = TestApp::spawn().await;

    let bare = environment(&app, json!({ "slug": "scratch" })).await;
    assert_eq!(bare["slug"], "scratch");
    assert_eq!(bare["name"], "scratch", "name defaults to the slug: {bare}");
    assert_eq!(bare["kind"], "other");
    assert_eq!(
        bare["data_state"], "unknown",
        "an environment nobody described is not the same as one described as empty: {bare}"
    );
    assert_eq!(bare["writable"], json!(true));
    assert!(bare["base_url"].is_null());
    assert!(bare["archived_at"].is_null());

    // Production errs toward refusing a destructive run rather than permitting
    // one. It is a default and not a rule, so a project that really does test
    // writes against production can still say so.
    let prod = environment(&app, json!({ "slug": "prod", "kind": "production" })).await;
    assert_eq!(
        prod["writable"],
        json!(false),
        "production defaults to read-only: {prod}"
    );

    let opted_in = environment(
        &app,
        json!({ "slug": "prod-write", "kind": "production", "writable": true }),
    )
    .await;
    assert_eq!(
        opted_in["writable"],
        json!(true),
        "an explicit choice wins over the default: {opted_in}"
    );

    // Everything a runner needs comes back on the read, so it never has to be
    // told any of this out of band.
    let full = environment(
        &app,
        json!({
            "slug": "staging",
            "name": "Staging (eu-west)",
            "kind": "staging",
            "base_url": "https://staging.example.com",
            "bring_up": "backlot up --ttl 900",
            "teardown": "backlot release",
            "data_state": "seeded",
            "credentials_hint": "env:STAGING_TOKEN",
            "notes": "reseeded 03:00 UTC; shares the payment sandbox",
        }),
    )
    .await;
    let (s, got) = app
        .get(
            &app.worker,
            &format!("/v1/environments/{}", full["id"].as_str().unwrap()),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{got}");
    assert_eq!(got["base_url"], "https://staging.example.com");
    assert_eq!(got["bring_up"], "backlot up --ttl 900");
    assert_eq!(got["teardown"], "backlot release");
    assert_eq!(got["credentials_hint"], "env:STAGING_TOKEN");

    // Listing is an envelope, like every other list route.
    let (s, list) = app.get(&app.worker, "/v1/projects/tp/environments").await;
    assert_eq!(s, StatusCode::OK, "{list}");
    assert_eq!(list["total"], json!(4), "{list}");
    let slugs: Vec<&str> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["slug"].as_str().unwrap())
        .collect();
    assert_eq!(
        slugs,
        vec!["prod", "prod-write", "scratch", "staging"],
        "sorted by slug, because the reader is picking one: {list}"
    );

    let (s, filtered) = app
        .get(&app.worker, "/v1/projects/tp/environments?kind=production")
        .await;
    assert_eq!(s, StatusCode::OK, "{filtered}");
    assert_eq!(filtered["total"], json!(2), "{filtered}");
}

/// A slug is the handle checks and tool calls carry, so it is unique per project
/// and can never be renamed. Both halves matter: without uniqueness "staging"
/// stops identifying anything, and with renaming every stored reference to it
/// breaks silently.
#[tokio::test]
async fn an_environment_slug_is_unique_and_cannot_be_renamed() {
    let app = TestApp::spawn().await;
    let first = environment(&app, json!({ "slug": "staging", "name": "Staging" })).await;

    let (s, dup) = app
        .post(
            &app.admin,
            "/v1/projects/tp/environments",
            json!({ "slug": "staging", "name": "Staging again" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{dup}");
    assert_eq!(dup["code"], "conflict.environment_slug");
    assert!(
        dup["message"]
            .as_str()
            .unwrap()
            .contains(first["id"].as_str().unwrap()),
        "the refusal names the environment already holding the slug: {dup}"
    );

    // Renaming is not a permitted operation rather than a discouraged one: the
    // field is not in the patch allowlist at all.
    let (s, renamed) = app
        .patch(
            &app.admin,
            &format!("/v1/environments/{}", first["id"].as_str().unwrap()),
            json!({ "slug": "staging2" }),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::BAD_REQUEST,
        "slug is not patchable: {renamed}"
    );

    // Everything else is.
    let (s, patched) = app
        .patch(
            &app.admin,
            &format!("/v1/environments/{}", first["id"].as_str().unwrap()),
            json!({ "name": "Staging (eu-west)", "kind": "staging", "notes": "reseeded nightly" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{patched}");
    assert_eq!(patched["name"], "Staging (eu-west)");
    assert_eq!(patched["kind"], "staging");
    assert_eq!(
        patched["version"],
        json!(2),
        "a patch bumps version: {patched}"
    );

    let (s, bad) = app
        .post(
            &app.admin,
            "/v1/projects/tp/environments",
            json!({ "slug": "Staging Prod" }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    assert_eq!(bad["code"], "validation.environment_slug");
}

/// `credentials_hint` points at where a credential lives and never holds one.
///
/// Every token with `read` can list environments, so a secret here would be a
/// secret handed to every reader. The PEM check is not secret detection and does
/// not pretend to be — it catches the single most common way a private key gets
/// pasted into a text box, and refuses it loudly enough to send the next person
/// to the docs.
#[tokio::test]
async fn a_credential_hint_is_a_pointer_and_a_pasted_key_is_refused() {
    let app = TestApp::spawn().await;

    for pointer in [
        "env:STAGING_TOKEN",
        "op://vault/staging/agent",
        "see the runbook at https://wiki.example.com/staging",
    ] {
        let env = environment(
            &app,
            json!({ "slug": format!("e{}", pointer.len()), "credentials_hint": pointer }),
        )
        .await;
        assert_eq!(env["credentials_hint"], pointer);
    }

    let (s, refused) = app
        .post(
            &app.admin,
            "/v1/projects/tp/environments",
            json!({
                "slug": "leaky",
                "credentials_hint": "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaA==\n",
            }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
    assert_eq!(refused["code"], "validation.environment_credentials_hint");
    assert!(
        refused["remedy"]
            .as_str()
            .unwrap()
            .contains("never stores credentials"),
        "the refusal has to say why, or it reads as an arbitrary limit: {refused}"
    );
}

/// Archiving is how an environment goes away, and it is reversible, because
/// every verdict ever taken there still references it. Deleting the row would
/// orphan that history — the same reason a check is archived rather than dropped.
#[tokio::test]
async fn archiving_an_environment_keeps_it_readable_and_is_reversible() {
    let app = TestApp::spawn().await;
    let env = environment(&app, json!({ "slug": "old-box", "kind": "shared" })).await;
    let id = env["id"].as_str().unwrap().to_string();

    let (s, archived) = app
        .delete(&app.admin, &format!("/v1/environments/{id}"))
        .await;
    assert_eq!(s, StatusCode::OK, "{archived}");
    assert!(archived["archived_at"].is_string(), "{archived}");

    // Gone from the default list, still readable by id, and still countable.
    let (_, list) = app.get(&app.worker, "/v1/projects/tp/environments").await;
    assert_eq!(
        list["total"],
        json!(0),
        "archived drops out by default: {list}"
    );
    let (_, with) = app
        .get(&app.worker, "/v1/projects/tp/environments?archived=include")
        .await;
    assert_eq!(with["total"], json!(1), "{with}");
    let (s, still) = app
        .get(&app.worker, &format!("/v1/environments/{id}"))
        .await;
    assert_eq!(
        s,
        StatusCode::OK,
        "a verdict's evidence stays readable: {still}"
    );

    // Archiving changed nothing else about it, so unarchiving is a pure reversal.
    let (s, back) = app
        .post(
            &app.admin,
            &format!("/v1/environments/{id}/unarchive"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{back}");
    assert!(back["archived_at"].is_null(), "{back}");
    assert_eq!(back["kind"], "shared", "nothing else moved: {back}");
}

/// The enums exist so a project cannot grow `prod`, `production` and `Prod` and
/// then be unable to answer "is it verified in production". Each refusal names
/// the values that would have worked.
#[tokio::test]
async fn environment_enums_and_urls_are_refused_with_the_valid_values() {
    let app = TestApp::spawn().await;

    for (field, value, code) in [
        ("kind", "prod", "validation.environment_kind"),
        ("data_state", "fresh", "validation.environment_data_state"),
    ] {
        let (s, b) = app
            .post(
                &app.admin,
                "/v1/projects/tp/environments",
                json!({ "slug": "e1", field: value }),
            )
            .await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{field}: {b}");
        assert_eq!(b["code"], code, "{b}");
        assert!(
            b["remedy"].as_str().unwrap().contains("Send one of:"),
            "a refusal that does not list the alternatives just costs a round trip: {b}"
        );
    }

    // A bare host is the common mistake, and it is the one that produces a
    // request to nowhere rather than an error, so it is refused up front.
    let (s, b) = app
        .post(
            &app.admin,
            "/v1/projects/tp/environments",
            json!({ "slug": "e2", "base_url": "staging.example.com" }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert_eq!(b["code"], "validation.environment_base_url");
}

// ---------------------------------------------------------------------------
// Checklist: releases, checks, cases, verdicts, policy, coverage, gate
// ---------------------------------------------------------------------------

/// Create a check and return its id.
async fn check(app: &TestApp, body: Value) -> String {
    let (status, b) = app.post(&app.admin, "/v1/projects/tp/checks", body).await;
    assert_eq!(status, StatusCode::CREATED, "check create failed: {b}");
    b["id"].as_str().expect("check id").to_string()
}

/// File a set of `(key, label)` cases on a check, replacing whatever was there.
async fn file_cases(app: &TestApp, check_id: &str, keys: &[&str]) -> Value {
    let cases: Vec<Value> = keys
        .iter()
        .map(|k| json!({ "key": k, "label": format!("case {k}"), "assignment": { "k": k } }))
        .collect();
    let (status, b) = app
        .put(
            &app.admin,
            &format!("/v1/checks/{check_id}/cases"),
            json!({ "cases": cases }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "file cases failed: {b}");
    b
}

async fn case_ids(app: &TestApp, check_id: &str) -> Vec<(String, String)> {
    let (_, b) = app
        .get(&app.admin, &format!("/v1/checks/{check_id}/cases"))
        .await;
    b["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["key"].as_str().unwrap().to_string(),
                c["id"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

/// Releases are an ordered spine: `seq` is monotonic per project so a
/// release-count policy is arithmetic, and a ref cannot be pushed twice — a
/// release is an immutable marker, not a mutable record.
#[tokio::test]
async fn releases_are_ordered_and_a_ref_cannot_be_pushed_twice() {
    let app = TestApp::spawn().await;

    let (status, first) = app
        .post(
            &app.admin,
            "/v1/projects/tp/releases",
            json!({ "ref": "v1.0.0", "touched_paths": ["src/a.rs"] }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    assert_eq!(first["seq"], 1);
    assert_eq!(first["touched_paths"], 1);

    let (status, second) = app
        .post(
            &app.admin,
            "/v1/projects/tp/releases",
            json!({ "ref": "v1.1.0" }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(second["seq"], 2, "seq increments per project");

    let (status, dup) = app
        .post(
            &app.admin,
            "/v1/projects/tp/releases",
            json!({ "ref": "v1.0.0" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{dup}");
    assert_eq!(dup["code"], "conflict.release_exists");
    assert!(
        !dup["remedy"].as_str().unwrap_or("").is_empty(),
        "a teaching error carries a remedy"
    );

    // Newest first.
    let (_, list) = app.get(&app.admin, "/v1/projects/tp/releases").await;
    let refs: Vec<&str> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["ref"].as_str().unwrap())
        .collect();
    assert_eq!(refs, vec!["v1.1.0", "v1.0.0"]);
}

/// The whole point of the glob claim: a release invalidates the checks that claim
/// the code it touched, and leaves the others alone. Getting this wrong either
/// invalidates everything (and the feature becomes noise) or nothing (and it
/// becomes a lie).
#[tokio::test]
async fn a_release_stales_only_the_checks_claiming_a_touched_path() {
    let app = TestApp::spawn().await;
    let claims = check(
        &app,
        json!({ "title": "Create a claim", "globs": ["src/claims/**"] }),
    )
    .await;
    let reports = check(
        &app,
        json!({ "title": "Monthly report", "globs": ["src/reporting/**"] }),
    )
    .await;
    file_cases(&app, &claims, &["a", "b"]).await;
    file_cases(&app, &reports, &["a"]).await;

    // Verify everything first, so "stale" is a real transition and not just "never".
    for check_id in [&claims, &reports] {
        for (_, cid) in case_ids(&app, check_id).await {
            let (status, b) = app
                .post(
                    &app.admin,
                    &format!("/v1/cases/{cid}/verdict"),
                    json!({ "verdict": "pass" }),
                )
                .await;
            assert_eq!(status, StatusCode::OK, "{b}");
        }
    }

    let (status, rel) = app
        .post(
            &app.admin,
            "/v1/projects/tp/releases",
            json!({ "ref": "r2", "touched_paths": ["src/claims/create.rs", "docs/readme.md"] }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{rel}");
    assert_eq!(
        rel["impact"]["stale_cases"], 2,
        "both claims cases go stale"
    );
    assert_eq!(
        rel["impact"]["stale_checks"].as_array().unwrap().len(),
        1,
        "only the check claiming src/claims/** is affected: {rel}"
    );

    let (_, claims_check) = app.get(&app.admin, &format!("/v1/checks/{claims}")).await;
    assert_eq!(claims_check["cases"]["stale"], 2);
    assert_eq!(claims_check["cases"]["verified"], 0);

    let (_, reports_check) = app.get(&app.admin, &format!("/v1/checks/{reports}")).await;
    assert_eq!(
        reports_check["cases"]["verified"], 1,
        "an untouched check keeps its verdicts: {reports_check}"
    );
}

/// Case identity is the assignment, not the row. Regenerating a model must match
/// surviving cases and keep their history; a case that disappears is retired, not
/// deleted, and one that comes back is revived with its verdicts intact.
#[tokio::test]
async fn refiling_cases_preserves_history_and_retires_the_absent() {
    let app = TestApp::spawn().await;
    let id = check(&app, json!({ "title": "Create a claim" })).await;
    let out = file_cases(&app, &id, &["k1", "k2"]).await;
    assert_eq!(out["added"], 2);

    let cases = case_ids(&app, &id).await;
    let k1 = cases.iter().find(|(k, _)| k == "k1").unwrap().1.clone();
    app.post(
        &app.admin,
        &format!("/v1/cases/{k1}/verdict"),
        json!({ "verdict": "pass", "note": "looked fine" }),
    )
    .await;

    // Regenerate with k2 gone and k3 added.
    let out = file_cases(&app, &id, &["k1", "k3"]).await;
    assert_eq!(out["added"], 1, "k3 is new: {out}");
    assert_eq!(out["updated"], 1, "k1 matched by key: {out}");
    assert_eq!(out["retired"], 1, "k2 is gone: {out}");
    assert_eq!(out["live"], 2);

    // k1 kept its id AND its verdict — that is the point of a stable key.
    let (_, k1_after) = app.get(&app.admin, &format!("/v1/cases/{k1}")).await;
    assert_eq!(k1_after["agent"]["verdict"], "pass");
    assert_eq!(k1_after["history"].as_array().unwrap().len(), 1);

    // k2 survives as history, and is refused for new work.
    let (_, with_retired) = app
        .get(
            &app.admin,
            &format!("/v1/checks/{id}/cases?retired=include"),
        )
        .await;
    let k2 = with_retired["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["key"] == "k2")
        .expect("k2 still exists as history");
    assert_eq!(k2["state"], "retired");
    let k2_id = k2["id"].as_str().unwrap();
    let (status, refused) = app
        .post(
            &app.admin,
            &format!("/v1/cases/{k2_id}/verdict"),
            json!({ "verdict": "pass" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["code"], "conflict.case_retired");

    // Bringing k2 back revives it rather than creating a second row.
    let out = file_cases(&app, &id, &["k1", "k2", "k3"]).await;
    assert_eq!(out["revived"], 1, "{out}");
    assert_eq!(out["added"], 0, "no duplicate row for the same assignment");
}

/// The agent's verdict and the human's are separate facts, because a policy of
/// `agent_then_human` needs both and collapsing them would make "a person looked
/// at this" unrecoverable.
#[tokio::test]
async fn agent_and_human_verdicts_are_separate_facts() {
    let app = TestApp::spawn().await;
    let id = check(
        &app,
        json!({ "title": "Create a claim", "verification": "agent_then_human" }),
    )
    .await;
    file_cases(&app, &id, &["only"]).await;
    let cid = case_ids(&app, &id).await[0].1.clone();

    let (_, after_agent) = app
        .post(
            &app.admin,
            &format!("/v1/cases/{cid}/verdict"),
            json!({ "verdict": "pass" }),
        )
        .await;
    assert_eq!(after_agent["agent"]["verdict"], "pass");
    assert!(after_agent["human"]["verdict"].is_null());
    assert_eq!(after_agent["state"], "verified");

    // Under agent_then_human the agent's pass is not enough — it still needs a
    // human, so the case stays on the worklist.
    let (_, wl) = app
        .get(&app.admin, "/v1/projects/tp/checklist/worklist")
        .await;
    assert_eq!(wl["human"]["cases"], 1, "awaiting a human: {wl}");
    assert_eq!(wl["agent"]["cases"], 0);
    assert_eq!(wl["human"]["items"][0]["reason"], "awaiting_human");

    let (_, after_human) = app
        .post(
            &app.human,
            &format!("/v1/cases/{cid}/verdict"),
            json!({ "verdict": "pass", "actor_kind": "human" }),
        )
        .await;
    assert_eq!(
        after_human["agent"]["verdict"], "pass",
        "the agent's fact survives a human verdict"
    );
    assert_eq!(after_human["human"]["verdict"], "pass");
    assert_eq!(after_human["state"], "approved");

    let (_, wl) = app
        .get(&app.admin, "/v1/projects/tp/checklist/worklist")
        .await;
    assert_eq!(wl["human"]["cases"], 0, "now cleared: {wl}");

    // Both verdicts are in the history, newest first.
    let (_, case) = app.get(&app.admin, &format!("/v1/cases/{cid}")).await;
    let kinds: Vec<&str> = case["history"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["actor_kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, vec!["human", "agent"]);
}

/// Two verdicts in the same millisecond still read newest-first.
///
/// This is the regression for the flake that turned `main` red: the history
/// tiebreak was `id DESC`, and a verdict id is `cv-` plus eight RANDOM base36
/// characters — so whenever both verdicts shared a millisecond, which is exactly
/// what an agent pass followed immediately by a human pass does, the order was a
/// coin flip. `agent_and_human_verdicts_are_separate_facts` asserts that order
/// and so failed about half the time.
///
/// Forcing the two timestamps equal makes the regression deterministic instead of
/// leaving it to chance: with a random tiebreak this fails roughly 50% of runs,
/// and with insertion order it cannot fail at all.
#[tokio::test]
async fn verdict_history_is_newest_first_within_a_single_millisecond() {
    let app = TestApp::spawn().await;
    let id = check(
        &app,
        json!({ "title": "Create a claim", "verification": "agent_then_human" }),
    )
    .await;
    file_cases(&app, &id, &["only"]).await;
    let cid = case_ids(&app, &id).await[0].1.clone();

    // Agent first, then human — so newest-first means human, agent.
    app.post(
        &app.admin,
        &format!("/v1/cases/{cid}/verdict"),
        json!({ "verdict": "pass" }),
    )
    .await;
    app.post(
        &app.human,
        &format!("/v1/cases/{cid}/verdict"),
        json!({ "verdict": "pass", "actor_kind": "human" }),
    )
    .await;

    // Collapse both timestamps onto one instant, so `at DESC` decides nothing and
    // the tiebreak is the only thing left ordering them.
    let conn = rusqlite::Connection::open(app.db_path()).expect("open db");
    let touched = conn
        .execute(
            "UPDATE case_verdicts SET at = 1000 WHERE case_id = ?1",
            rusqlite::params![cid],
        )
        .expect("flatten timestamps");
    assert_eq!(touched, 2, "both verdicts should have been recorded");

    let (_, case) = app.get(&app.admin, &format!("/v1/cases/{cid}")).await;
    let kinds: Vec<&str> = case["history"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["actor_kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        vec!["human", "agent"],
        "with equal timestamps the tiebreak must be insertion order, not a random \
         id: {case}"
    );
}

/// An agent must not be able to sign a person's name. Recording a human verdict
/// needs the `human` scope; a write-scoped agent token is refused.
#[tokio::test]
async fn a_human_verdict_needs_the_human_scope() {
    let app = TestApp::spawn().await;
    let id = check(&app, json!({ "title": "Create a claim" })).await;
    file_cases(&app, &id, &["only"]).await;
    let cid = case_ids(&app, &id).await[0].1.clone();

    let (status, refused) = app
        .post(
            &app.worker,
            &format!("/v1/cases/{cid}/verdict"),
            json!({ "verdict": "pass", "actor_kind": "human" }),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    assert_eq!(refused["code"], "forbidden.human_scope");
    assert!(refused["remedy"]
        .as_str()
        .unwrap_or("")
        .contains("actor_kind"));

    // The same agent may record its own observation.
    let (status, _) = app
        .post(
            &app.worker,
            &format!("/v1/cases/{cid}/verdict"),
            json!({ "verdict": "pass" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}

/// A bare `fail` is a claim the next reader cannot act on.
#[tokio::test]
async fn a_fail_verdict_requires_a_note() {
    let app = TestApp::spawn().await;
    let id = check(&app, json!({ "title": "Create a claim" })).await;
    file_cases(&app, &id, &["only"]).await;
    let cid = case_ids(&app, &id).await[0].1.clone();

    let (status, refused) = app
        .post(
            &app.admin,
            &format!("/v1/cases/{cid}/verdict"),
            json!({ "verdict": "fail" }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
    assert_eq!(refused["code"], "validation.verdict_note");

    let (status, ok) = app
        .post(
            &app.admin,
            &format!("/v1/cases/{cid}/verdict"),
            json!({ "verdict": "fail", "note": "submit was accepted when it should not be" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ok["state"], "failed");
}

/// Policy resolves project → epic → check, and each level says where its value
/// came from — an inherited setting nobody can trace is worse than no setting.
#[tokio::test]
async fn policy_resolves_project_then_epic_then_check() {
    let app = TestApp::spawn().await;
    let epic = app.create_typed("Claims", "epic", None).await;

    let plain = check(&app, json!({ "title": "Ungrouped" })).await;
    let (_, l) = app.get(&app.admin, &format!("/v1/checks/{plain}")).await;
    assert_eq!(l["policy"]["verification"], "agent", "built-in default");
    assert_eq!(l["policy"]["verification_from"], "default");

    // Project default.
    let (status, _) = app
        .put(
            &app.admin,
            "/v1/projects/tp/checklist/policy",
            json!({ "verification": "human", "expiry_releases": 5 }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, l) = app.get(&app.admin, &format!("/v1/checks/{plain}")).await;
    assert_eq!(l["policy"]["verification"], "human");
    assert_eq!(l["policy"]["verification_from"], "project");
    assert_eq!(l["policy"]["expiry_releases"], 5);

    // Epic override beats the project.
    app.put(
        &app.admin,
        "/v1/projects/tp/checklist/policy",
        json!({ "epic": epic, "verification": "agent_then_human" }),
    )
    .await;
    let grouped = check(&app, json!({ "title": "In the epic", "epic": epic })).await;
    let (_, l) = app.get(&app.admin, &format!("/v1/checks/{grouped}")).await;
    assert_eq!(l["policy"]["verification"], "agent_then_human");
    assert_eq!(l["policy"]["verification_from"], "epic");

    // Check override beats both.
    app.patch(
        &app.admin,
        &format!("/v1/checks/{grouped}"),
        json!({ "verification": "agent" }),
    )
    .await;
    let (_, l) = app.get(&app.admin, &format!("/v1/checks/{grouped}")).await;
    assert_eq!(l["policy"]["verification"], "agent");
    assert_eq!(l["policy"]["verification_from"], "check");

    // Explicit null clears the override and inheritance resumes. This is why the
    // wire format has to distinguish absent from null.
    app.patch(
        &app.admin,
        &format!("/v1/checks/{grouped}"),
        json!({ "verification": null }),
    )
    .await;
    let (_, l) = app.get(&app.admin, &format!("/v1/checks/{grouped}")).await;
    assert_eq!(l["policy"]["verification"], "agent_then_human");
    assert_eq!(l["policy"]["verification_from"], "epic");
}

/// Release-count expiry: verified at r1 with a limit of 1 release, so pushing r2
/// ages it out even though nothing in the diff touched the check.
#[tokio::test]
async fn release_count_expiry_stales_a_case_without_a_touching_diff() {
    let app = TestApp::spawn().await;
    let id = check(
        &app,
        json!({ "title": "Create a claim", "globs": ["src/claims/**"], "expiry_releases": 1 }),
    )
    .await;
    file_cases(&app, &id, &["only"]).await;
    let cid = case_ids(&app, &id).await[0].1.clone();

    let (_, r1) = app
        .post(
            &app.admin,
            "/v1/projects/tp/releases",
            json!({ "ref": "r1" }),
        )
        .await;
    let r1_id = r1["id"].as_str().unwrap();
    app.post(
        &app.admin,
        &format!("/v1/cases/{cid}/verdict"),
        json!({ "verdict": "pass", "release": r1_id }),
    )
    .await;

    let (_, r2) = app
        .post(
            &app.admin,
            "/v1/projects/tp/releases",
            json!({ "ref": "r2", "touched_paths": ["docs/unrelated.md"] }),
        )
        .await;
    assert_eq!(
        r2["impact"]["stale_cases"], 1,
        "the policy clock ran out even though the diff missed the check: {r2}"
    );
    assert_eq!(r2["impact"]["expired_checks"].as_array().unwrap().len(), 1);

    let (_, wl) = app
        .get(&app.admin, "/v1/projects/tp/checklist/worklist")
        .await;
    assert_eq!(wl["agent"]["items"][0]["reason"], "stale");
}

/// Time-based expiry, driven by backdating the verdict in the store rather than
/// sleeping. Both expiry kinds apply and whichever trips first wins.
#[tokio::test]
async fn time_based_expiry_puts_a_case_back_on_the_worklist() {
    let app = TestApp::spawn().await;
    let id = check(
        &app,
        json!({ "title": "Create a claim", "expiry_days": 30 }),
    )
    .await;
    file_cases(&app, &id, &["only"]).await;
    let cid = case_ids(&app, &id).await[0].1.clone();
    app.post(
        &app.admin,
        &format!("/v1/cases/{cid}/verdict"),
        json!({ "verdict": "pass" }),
    )
    .await;

    let (_, wl) = app
        .get(&app.admin, "/v1/projects/tp/checklist/worklist")
        .await;
    assert_eq!(wl["agent"]["cases"], 0, "fresh, so nothing to do: {wl}");

    // 45 days ago — past the 30-day policy.
    app.backdate_case_verdict(&cid, 45 * 86_400_000);

    let (_, wl) = app
        .get(&app.admin, "/v1/projects/tp/checklist/worklist")
        .await;
    assert_eq!(wl["agent"]["cases"], 1, "aged out: {wl}");
    assert_eq!(wl["agent"]["items"][0]["reason"], "expired");
}

/// `unreachable` is counted apart from covered and uncovered. Calling it a gap
/// reports work nobody can do; calling it covered claims verification of code no
/// path reaches.
#[tokio::test]
async fn coverage_counts_unreachable_apart_and_reports_orphaned_globs() {
    let app = TestApp::spawn().await;
    let epic = app.create_typed("Claims", "epic", None).await;
    let id = check(
        &app,
        json!({
            "title": "Create a claim",
            "epic": epic,
            "layer": "ui",
            "globs": ["src/claims/**", "src/claims/legacy/**"],
        }),
    )
    .await;
    file_cases(&app, &id, &["a", "b", "c", "d"]).await;
    let cases = case_ids(&app, &id).await;

    let verdicts = [("a", "pass"), ("b", "unreachable"), ("c", "pass")];
    for (key, verdict) in verdicts {
        let cid = &cases.iter().find(|(k, _)| k == key).unwrap().1;
        app.post(
            &app.admin,
            &format!("/v1/cases/{cid}/verdict"),
            json!({ "verdict": verdict }),
        )
        .await;
    }

    let (status, cov) = app
        .get(&app.admin, "/v1/projects/tp/checklist/coverage")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cov["cases"]["total"], 4);
    assert_eq!(cov["cases"]["verified"], 2);
    assert_eq!(cov["cases"]["unreachable"], 1);
    assert_eq!(cov["cases"]["never"], 1);
    // 2 verified of 3 verifiable cases. The unreachable one is out of BOTH the
    // numerator and the denominator: leaving it in the denominator would cap this
    // project below 100% forever with no action that could close the gap.
    assert_eq!(cov["percent"], 66, "{cov}");
    assert_eq!(cov["epics"][0]["epic"], epic);
    assert_eq!(cov["epics"][0]["checks"], 1);

    // An orphaned glob is flagged rather than counted, because a check claiming
    // code that is not there reads as covered while covering nothing.
    app.post(
        &app.admin,
        "/v1/projects/tp/releases",
        json!({
            "ref": "r1",
            "touched_paths": ["docs/x.md"],
            "orphan_globs": ["src/claims/legacy/**"],
        }),
    )
    .await;
    let (_, l) = app.get(&app.admin, &format!("/v1/checks/{id}")).await;
    assert_eq!(
        l["orphan_globs"],
        json!(["src/claims/legacy/**"]),
        "the empty glob is surfaced on the check: {l}"
    );
}

/// The gate blocks on `blocking` severity only. Advisory and low checks nag: a
/// gate that fires on everything gets overridden out of habit and stops meaning
/// anything.
#[tokio::test]
async fn the_gate_blocks_on_blocking_severity_and_nags_on_the_rest() {
    let app = TestApp::spawn().await;
    let advisory = check(
        &app,
        json!({ "title": "Print documents", "severity": "advisory" }),
    )
    .await;
    file_cases(&app, &advisory, &["a"]).await;

    let (_, gate) = app.get(&app.admin, "/v1/projects/tp/checklist/gate").await;
    assert_eq!(
        gate["blocked"], false,
        "advisory alone never blocks: {gate}"
    );
    assert_eq!(gate["advisory_outstanding"], 1);

    let blocking = check(
        &app,
        json!({ "title": "Create a claim", "severity": "blocking" }),
    )
    .await;
    file_cases(&app, &blocking, &["a"]).await;
    let (_, gate) = app.get(&app.admin, "/v1/projects/tp/checklist/gate").await;
    assert_eq!(gate["blocked"], true, "{gate}");
    assert_eq!(gate["blocking"]["agent_cases"], 1);

    // Clearing the blocking case unblocks it; the advisory one still only nags.
    let cid = case_ids(&app, &blocking).await[0].1.clone();
    app.post(
        &app.admin,
        &format!("/v1/cases/{cid}/verdict"),
        json!({ "verdict": "pass" }),
    )
    .await;
    let (_, gate) = app.get(&app.admin, "/v1/projects/tp/checklist/gate").await;
    assert_eq!(gate["blocked"], false, "{gate}");
    assert_eq!(gate["advisory_outstanding"], 1);
}

/// The worklist splits by who can clear an item, because human time is the scarce
/// resource. A stale case under `agent_then_human` needs the agent first, so it
/// must not sit in a person's queue waiting for work only an agent can do.
#[tokio::test]
async fn the_worklist_routes_agent_first_work_away_from_humans() {
    let app = TestApp::spawn().await;
    let id = check(
        &app,
        json!({
            "title": "Create a claim",
            "verification": "agent_then_human",
            "globs": ["src/claims/**"],
            "cost_agent_minutes": 2,
            "cost_human_minutes": 15,
        }),
    )
    .await;
    file_cases(&app, &id, &["a"]).await;
    let cid = case_ids(&app, &id).await[0].1.clone();

    // Never verified: the agent has to go first.
    let (_, wl) = app
        .get(&app.admin, "/v1/projects/tp/checklist/worklist")
        .await;
    assert_eq!(wl["agent"]["cases"], 1, "{wl}");
    assert_eq!(wl["human"]["cases"], 0);
    assert_eq!(wl["agent"]["minutes"], 2, "costed with the agent estimate");

    // Agent passes, so now it needs the human — with the human cost.
    app.post(
        &app.admin,
        &format!("/v1/cases/{cid}/verdict"),
        json!({ "verdict": "pass" }),
    )
    .await;
    let (_, wl) = app
        .get(&app.admin, "/v1/projects/tp/checklist/worklist")
        .await;
    assert_eq!(wl["agent"]["cases"], 0);
    assert_eq!(wl["human"]["cases"], 1, "{wl}");
    assert_eq!(wl["human"]["minutes"], 15);

    app.post(
        &app.human,
        &format!("/v1/cases/{cid}/verdict"),
        json!({ "verdict": "pass", "actor_kind": "human" }),
    )
    .await;

    // A release touching the claimed code sends it back to the AGENT, not to the
    // human who just signed it off.
    app.post(
        &app.admin,
        "/v1/projects/tp/releases",
        json!({ "ref": "r1", "touched_paths": ["src/claims/create.rs"] }),
    )
    .await;
    let (_, wl) = app
        .get(&app.admin, "/v1/projects/tp/checklist/worklist")
        .await;
    assert_eq!(
        wl["agent"]["cases"], 1,
        "stale under agent_then_human is agent work first: {wl}"
    );
    assert_eq!(wl["human"]["cases"], 0);
}

/// Checks group under a `type: epic` ticket so the vocabulary matches tickets.
/// Anything else is a typo worth a loud refusal rather than a silent orphan.
#[tokio::test]
async fn a_check_refuses_a_parent_that_is_not_an_epic() {
    let app = TestApp::spawn().await;
    let task = app.create_typed("Just a task", "task", None).await;
    let (status, refused) = app
        .post(
            &app.admin,
            "/v1/projects/tp/checks",
            json!({ "title": "Create a claim", "epic": task }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
    assert_eq!(refused["code"], "validation.check_epic");

    let (status, refused) = app
        .post(
            &app.admin,
            "/v1/projects/tp/checks",
            json!({ "title": "Create a claim", "epic": "nope-9999" }),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{refused}");
}

/// Enum and shape validation is part of the contract: a typo must be a loud 4xx
/// carrying the legal values, never a silently dropped field.
#[tokio::test]
async fn check_and_case_input_is_validated_with_teaching_errors() {
    let app = TestApp::spawn().await;

    let (status, bad) = app
        .post(
            &app.admin,
            "/v1/projects/tp/checks",
            json!({ "title": "X", "layer": "gui" }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    assert_eq!(bad["code"], "validation.check_layer");
    assert!(
        bad["message"].as_str().unwrap().contains("ui"),
        "the error names the legal values: {bad}"
    );

    let (status, bad) = app
        .post(
            &app.admin,
            "/v1/projects/tp/checks",
            json!({ "title": "X", "sevrity": "blocking" }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a typo'd field is loud: {bad}"
    );

    let id = check(&app, json!({ "title": "Create a claim" })).await;
    let (status, bad) = app
        .put(
            &app.admin,
            &format!("/v1/checks/{id}/cases"),
            json!({ "cases": [{ "key": "dup" }, { "key": "dup" }] }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    assert_eq!(bad["code"], "validation.case_key");

    // A missing key is caught by the shared body parser before the store sees it,
    // so it is a 400 on the field rather than a 422 on the concept. Either way it
    // names the field: a case without a stable key would silently break history.
    let (status, bad) = app
        .put(
            &app.admin,
            &format!("/v1/checks/{id}/cases"),
            json!({ "cases": [{ "label": "no key" }] }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    assert_eq!(bad["code"], "validation.field_required");
    assert!(bad["message"].as_str().unwrap().contains("key"), "{bad}");
}

/// Archiving keeps the evidence. A check no longer worth running is still a record
/// of what was once verified, so it leaves coverage without deleting history.
#[tokio::test]
async fn archiving_a_check_drops_it_from_coverage_but_keeps_its_cases() {
    let app = TestApp::spawn().await;
    let id = check(&app, json!({ "title": "Create a claim" })).await;
    file_cases(&app, &id, &["a"]).await;

    let (_, cov) = app
        .get(&app.admin, "/v1/projects/tp/checklist/coverage")
        .await;
    assert_eq!(cov["checks"], 1);

    let (status, archived) = app.delete(&app.admin, &format!("/v1/checks/{id}")).await;
    assert_eq!(status, StatusCode::OK, "{archived}");
    assert!(!archived["archived_at"].is_null());

    let (_, cov) = app
        .get(&app.admin, "/v1/projects/tp/checklist/coverage")
        .await;
    assert_eq!(cov["checks"], 0, "archived checks leave coverage: {cov}");

    let (_, checks) = app.get(&app.admin, "/v1/projects/tp/checks").await;
    assert_eq!(checks["items"].as_array().unwrap().len(), 0);
    let (_, checks) = app
        .get(&app.admin, "/v1/projects/tp/checks?archived=include")
        .await;
    assert_eq!(checks["items"].as_array().unwrap().len(), 1);

    // The cases and their history are still reachable.
    let (status, cases) = app.get(&app.admin, &format!("/v1/checks/{id}/cases")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cases["items"].as_array().unwrap().len(), 1, "{cases}");
}

/// A verdict against a release from another project is a wiring mistake worth
/// catching, not a null to store.
#[tokio::test]
async fn a_verdict_cannot_cite_an_unknown_release() {
    let app = TestApp::spawn().await;
    let id = check(&app, json!({ "title": "Create a claim" })).await;
    file_cases(&app, &id, &["a"]).await;
    let cid = case_ids(&app, &id).await[0].1.clone();

    let (status, bad) = app
        .post(
            &app.admin,
            &format!("/v1/cases/{cid}/verdict"),
            json!({ "verdict": "pass", "release": "rel-nope" }),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{bad}");
}

/// A release must not turn "never tested" into "stale". Stale means *was verified,
/// then the code moved*; using it for work nobody has done once would shrink the
/// never-tested gap this feature exists to expose.
#[tokio::test]
async fn a_release_does_not_stale_a_case_that_was_never_verified() {
    let app = TestApp::spawn().await;
    let id = check(
        &app,
        json!({ "title": "Create a claim", "globs": ["src/claims/**"] }),
    )
    .await;
    file_cases(&app, &id, &["verified", "untouched"]).await;
    let cases = case_ids(&app, &id).await;
    let verified = cases
        .iter()
        .find(|(k, _)| k == "verified")
        .unwrap()
        .1
        .clone();
    app.post(
        &app.admin,
        &format!("/v1/cases/{verified}/verdict"),
        json!({ "verdict": "pass" }),
    )
    .await;

    let (_, rel) = app
        .post(
            &app.admin,
            "/v1/projects/tp/releases",
            json!({ "ref": "r1", "touched_paths": ["src/claims/create.rs"] }),
        )
        .await;
    assert_eq!(
        rel["impact"]["stale_cases"], 1,
        "only the verified case goes stale: {rel}"
    );

    let (_, l) = app.get(&app.admin, &format!("/v1/checks/{id}")).await;
    assert_eq!(l["cases"]["stale"], 1);
    assert_eq!(
        l["cases"]["never"], 1,
        "the never-verified case is still reported as never: {l}"
    );

    // And the worklist distinguishes the two reasons, because they call for
    // different work: re-run versus write-and-run.
    let (_, wl) = app
        .get(&app.admin, "/v1/projects/tp/checklist/worklist")
        .await;
    let reasons: Vec<&str> = wl["agent"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["reason"].as_str().unwrap())
        .collect();
    assert!(reasons.contains(&"stale"), "{wl}");
    assert!(reasons.contains(&"never"), "{wl}");
}

/// An initiative can answer whether the tests it agreed on still pass.
///
/// This is the link the whole surface is for: a characterisation test gets
/// settled while a feature is being discussed, and months later somebody has to
/// know whether it still holds. The reference is DIRECT rather than derived
/// through the epic's `initiative:` tag, because the agreement usually predates
/// any epic — deriving it would make the link unstateable exactly when it is made.
#[tokio::test]
async fn an_initiative_reports_the_standing_of_the_checks_it_agreed() {
    let app = TestApp::spawn().await;
    let ini = new_initiative(&app, "Split billing", "open").await;
    let other = new_initiative(&app, "Something else", "open").await;

    // Filed against the initiative with no epic in sight — the case this exists for.
    let split = check(
        &app,
        json!({
            "title": "Split an invoice",
            "initiative": ini,
            "severity": "blocking",
            "globs": ["src/billing/**"],
        }),
    )
    .await;
    let unattached = check(&app, json!({ "title": "Nobody claimed this one" })).await;
    let elsewhere = check(&app, json!({ "title": "Other work", "initiative": other })).await;

    file_cases(&app, &split, &["a", "b", "c"]).await;
    let cases = case_ids(&app, &split).await;

    // Two verified, one never run — the spread the rollup has to report.
    for (_key, id) in cases.iter().take(2) {
        let (s, b) = app
            .post(
                &app.admin,
                &format!("/v1/cases/{id}/verdict"),
                json!({ "verdict": "pass" }),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{b}");
    }

    let (s, v) = app
        .get(&app.admin, &format!("/v1/initiatives/{ini}/verification"))
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["title"], "Split billing");
    assert_eq!(v["checks"], json!(1), "only its own checks: {v}");
    assert_eq!(v["cases"]["verified"], json!(2), "{v}");
    assert_eq!(
        v["cases"]["never"],
        json!(1),
        "a case nobody ran is the gap this exists to show: {v}"
    );
    assert!(
        v["last_verified_at"].is_string(),
        "'when was this last checked' is the other half of the question: {v}"
    );
    assert_eq!(
        v["blocked"],
        json!(true),
        "a blocking check with a never-run case is not ready to ship: {v}"
    );

    // The other initiative's checks are its own, and the unattached one belongs
    // to neither — a check nobody filed under an initiative is a fact, not a gap
    // to be papered over by guessing.
    let (_, o) = app
        .get(&app.admin, &format!("/v1/initiatives/{other}/verification"))
        .await;
    assert_eq!(o["checks"], json!(1), "{o}");
    assert_eq!(o["cases"]["total"], json!(0), "{o}");

    // …and the list route can find both readings.
    let (_, mine) = app
        .get(
            &app.admin,
            &format!("/v1/projects/tp/checks?initiative={ini}"),
        )
        .await;
    assert_eq!(mine["total"], json!(1), "{mine}");
    assert_eq!(mine["items"][0]["id"], json!(split));
    assert_eq!(mine["items"][0]["initiative"], json!(ini));

    let (_, orphans) = app
        .get(&app.admin, "/v1/projects/tp/checks?initiative=none")
        .await;
    assert_eq!(
        orphans["total"],
        json!(1),
        "'none' finds what nothing agreed: {orphans}"
    );
    assert_eq!(orphans["items"][0]["id"], json!(unattached));

    // The link is editable in both directions, and an explicit null detaches.
    let (s, moved) = app
        .patch(
            &app.admin,
            &format!("/v1/checks/{elsewhere}"),
            json!({ "initiative": ini }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{moved}");
    assert_eq!(moved["initiative"], json!(ini));
    let (s, detached) = app
        .patch(
            &app.admin,
            &format!("/v1/checks/{elsewhere}"),
            json!({ "initiative": null }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{detached}");
    assert!(detached["initiative"].is_null(), "{detached}");
}

/// A wrong initiative id is refused where it is made.
///
/// Deliberately unlike a ticket's `initiative:` TAG, which the roadmap lets
/// dangle and reports under `uninitiated` — a tag is typed by hand, while this
/// is a column chosen from a list, so a mistake here is worth a teaching 422
/// rather than a row that quietly belongs to nothing.
#[tokio::test]
async fn a_check_refuses_an_initiative_that_does_not_exist_or_is_elsewhere() {
    let app = TestApp::spawn().await;

    let (s, b) = app
        .post(
            &app.admin,
            "/v1/projects/tp/checks",
            json!({ "title": "Orphan", "initiative": "ini-nosuchid" }),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{b}");
    assert_eq!(b["code"], "notfound.initiative");

    // One that exists, but in another project.
    let (s, other_project) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({ "id": "op", "name": "Other project" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{other_project}");
    let (s, foreign) = app
        .post(
            &app.admin,
            "/v1/initiatives",
            json!({ "project": "op", "title": "Theirs" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{foreign}");

    let (s, b) = app
        .post(
            &app.admin,
            "/v1/projects/tp/checks",
            json!({ "title": "Crossed wires", "initiative": foreign["id"] }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert_eq!(b["code"], "validation.check_initiative");
}

/// "Nobody ran it" and "we ran it and could not judge" are different findings.
///
/// `blocked` was an accepted verdict with nowhere to land: no state, no bucket,
/// so a case whose only verdict was blocked read as `never` everywhere and the
/// effort that went into it vanished. That is the wrong way round — a blocked
/// case usually means the environment needs fixing, while a never-run one means
/// the work has not started, and they go to different people.
#[tokio::test]
async fn a_blocked_case_is_not_a_case_nobody_ran() {
    let app = TestApp::spawn().await;
    let id = check(
        &app,
        json!({ "title": "Split an invoice", "severity": "blocking" }),
    )
    .await;
    file_cases(&app, &id, &["a", "b"]).await;
    let cases = case_ids(&app, &id).await;
    let (_, blocked_case) = cases.iter().find(|(k, _)| k == "a").expect("case a");

    let (s, b) = app
        .post(
            &app.admin,
            &format!("/v1/cases/{blocked_case}/verdict"),
            json!({ "verdict": "blocked", "note": "the seed data never loaded" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(
        b["state"], "blocked",
        "a blocked verdict has to be visible as its own state: {b}"
    );

    // It counts apart from `never`, and stays inside the coverage denominator —
    // a blocked case IS verifiable, it just was not verified this time.
    let (_, c) = app.get(&app.admin, &format!("/v1/checks/{id}")).await;
    assert_eq!(c["cases"]["blocked"], json!(1), "{c}");
    assert_eq!(
        c["cases"]["never"],
        json!(1),
        "the untouched case is the only `never`: {c}"
    );
    assert_eq!(c["cases"]["total"], json!(2), "{c}");

    // The worklist calls it what it is, rather than routing it to a person as
    // though it were waiting on their signature.
    let (_, wl) = app
        .get(&app.admin, "/v1/projects/tp/checklist/worklist")
        .await;
    let reason = wl["agent"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["case"] == json!(blocked_case))
        .map(|i| i["reason"].clone())
        .expect("the blocked case is on the agent worklist");
    assert_eq!(reason, json!("blocked"), "{wl}");

    // And it does not satisfy the policy, so a blocking check with one stays
    // shut — "could not judge" is not "fine".
    let (_, gate) = app.get(&app.admin, "/v1/projects/tp/checklist/gate").await;
    assert_eq!(gate["blocked"], json!(true), "{gate}");

    // A real pass still wins over a blocked verdict recorded earlier: the pass
    // is evidence about correctness and the blocked one never was.
    let (s, passed) = app
        .post(
            &app.admin,
            &format!("/v1/cases/{blocked_case}/verdict"),
            json!({ "verdict": "pass" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{passed}");
    assert_eq!(passed["state"], "verified", "{passed}");
}

/// A check declaring two environments tracks each one separately.
///
/// This is the whole point of declaring them: "passes on staging, never run on
/// production" has to be expressible, and the case's own word has to be the
/// WORST of its environments — otherwise the feature reports the opposite of
/// what it exists to show.
#[tokio::test]
async fn a_case_is_verified_per_environment_and_the_worst_one_wins() {
    let app = TestApp::spawn().await;
    let staging = environment(&app, json!({ "slug": "staging", "kind": "staging" })).await;
    let prod = environment(&app, json!({ "slug": "prod", "kind": "production" })).await;

    let id = check(
        &app,
        json!({
            "title": "Split an invoice",
            "environments": ["staging", "prod"],
            "severity": "blocking",
        }),
    )
    .await;
    let (_, c) = app.get(&app.admin, &format!("/v1/checks/{id}")).await;
    let slugs: Vec<&str> = c["environments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["slug"].as_str().unwrap())
        .collect();
    assert_eq!(slugs, vec!["prod", "staging"], "declared, by slug: {c}");

    file_cases(&app, &id, &["a"]).await;
    let case = case_ids(&app, &id).await[0].1.clone();

    // Nothing recorded anywhere: every declared environment reads `never`, and
    // so does the case.
    let (_, before) = app.get(&app.admin, &format!("/v1/cases/{case}")).await;
    assert_eq!(before["state"], "never", "{before}");
    assert_eq!(before["environments"].as_array().unwrap().len(), 2);
    for e in before["environments"].as_array().unwrap() {
        assert_eq!(e["state"], "never", "{e}");
    }

    // Pass on staging only.
    let (s, b) = app
        .post(
            &app.admin,
            &format!("/v1/cases/{case}/verdict"),
            json!({ "verdict": "pass", "environment": "staging" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(
        b["state"], "never",
        "staging alone does not verify the case — production has never been run: {b}"
    );
    let by_slug: std::collections::HashMap<&str, &Value> = b["environments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["slug"].as_str().unwrap(), e))
        .collect();
    assert_eq!(by_slug["staging"]["state"], "verified", "{b}");
    assert_eq!(by_slug["prod"]["state"], "never", "{b}");

    // The check's headline counts one row per CASE (the worst reading), and
    // reports the same set broken out per environment beside it.
    let (_, ck) = app.get(&app.admin, &format!("/v1/checks/{id}")).await;
    assert_eq!(ck["cases"]["never"], json!(1), "{ck}");
    assert_eq!(
        ck["cases"]["total"],
        json!(1),
        "not doubled by a second environment: {ck}"
    );
    let env_cases: std::collections::HashMap<&str, &Value> = ck["environment_cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["slug"].as_str().unwrap(), e))
        .collect();
    assert_eq!(env_cases["staging"]["cases"]["verified"], json!(1), "{ck}");
    assert_eq!(env_cases["prod"]["cases"]["never"], json!(1), "{ck}");

    // Now production too — and only then is the case verified.
    let (_, done) = app
        .post(
            &app.admin,
            &format!("/v1/cases/{case}/verdict"),
            json!({ "verdict": "pass", "environment": prod["slug"] }),
        )
        .await;
    assert_eq!(done["state"], "verified", "{done}");

    // The history says where each verdict was taken, which is what makes an old
    // one auditable rather than merely present.
    let (_, full) = app.get(&app.admin, &format!("/v1/cases/{case}")).await;
    let envs: Vec<&str> = full["history"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["environment"].as_str().unwrap_or("?"))
        .collect();
    assert_eq!(envs.len(), 2, "{full}");
    assert!(
        envs.contains(&staging["id"].as_str().unwrap())
            && envs.contains(&prod["id"].as_str().unwrap()),
        "every verdict records its environment: {full}"
    );
}

/// A verdict that does not say where it was taken is refused when that is
/// genuinely ambiguous — and resolved when it is not.
///
/// The silent alternative is the one failure this design exists to prevent: a
/// staging run filed as production is worse than no record at all. Resolving the
/// single-environment case keeps every caller written before this existed
/// working unchanged.
#[tokio::test]
async fn an_omitted_environment_resolves_when_unambiguous_and_refuses_when_not() {
    let app = TestApp::spawn().await;
    environment(&app, json!({ "slug": "staging" })).await;
    environment(&app, json!({ "slug": "prod" })).await;

    // A check with no environments: the original behaviour, unchanged.
    let plain = check(&app, json!({ "title": "Anywhere" })).await;
    file_cases(&app, &plain, &["a"]).await;
    let plain_case = case_ids(&app, &plain).await[0].1.clone();
    let (s, b) = app
        .post(
            &app.admin,
            &format!("/v1/cases/{plain_case}/verdict"),
            json!({ "verdict": "pass" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["state"], "verified", "{b}");
    assert!(
        b["environments"].as_array().unwrap().is_empty(),
        "environment-agnostic stays that way: {b}"
    );
    // …and naming one there is refused rather than stored where nothing reads it.
    let (s, b) = app
        .post(
            &app.admin,
            &format!("/v1/cases/{plain_case}/verdict"),
            json!({ "verdict": "pass", "environment": "staging" }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert_eq!(b["code"], "validation.verdict_environment");

    // Exactly one declared: an omitted environment means that one.
    let one = check(
        &app,
        json!({ "title": "Only staging", "environments": ["staging"] }),
    )
    .await;
    file_cases(&app, &one, &["a"]).await;
    let one_case = case_ids(&app, &one).await[0].1.clone();
    let (s, b) = app
        .post(
            &app.admin,
            &format!("/v1/cases/{one_case}/verdict"),
            json!({ "verdict": "pass" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["state"], "verified", "{b}");
    assert_eq!(b["environments"][0]["slug"], "staging", "{b}");

    // Two declared: refused, naming both so the caller can pick.
    let two = check(
        &app,
        json!({ "title": "Both", "environments": ["staging", "prod"] }),
    )
    .await;
    file_cases(&app, &two, &["a"]).await;
    let two_case = case_ids(&app, &two).await[0].1.clone();
    let (s, b) = app
        .post(
            &app.admin,
            &format!("/v1/cases/{two_case}/verdict"),
            json!({ "verdict": "pass" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert_eq!(b["code"], "conflict.environment_ambiguous");
    let named: Vec<&str> = b["details"]["environments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        named.contains(&"staging") && named.contains(&"prod"),
        "the refusal names the candidates, or it costs a round trip: {b}"
    );

    // An environment the check does not declare is refused too: a verdict is
    // evidence about the check, so it has to be taken where the check claims.
    environment(&app, json!({ "slug": "elsewhere" })).await;
    let (s, b) = app
        .post(
            &app.admin,
            &format!("/v1/cases/{two_case}/verdict"),
            json!({ "verdict": "pass", "environment": "elsewhere" }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert_eq!(b["code"], "validation.verdict_environment");
}

/// A release stales every environment's reading, and the worklist says where.
///
/// The release is a fact about CODE: the thing that was verified no longer
/// exists, wherever it was verified. And `stale_cases` keeps counting DISTINCT
/// cases while `stale_pairs` counts the work, so a caller counting cases does
/// not see one check's work reported twice over.
#[tokio::test]
async fn a_release_stales_every_environment_and_the_worklist_says_where() {
    let app = TestApp::spawn().await;
    environment(
        &app,
        json!({ "slug": "staging", "base_url": "https://staging.example.com" }),
    )
    .await;
    environment(&app, json!({ "slug": "prod" })).await;

    let id = check(
        &app,
        json!({
            "title": "Split an invoice",
            "environments": ["staging", "prod"],
            "globs": ["src/billing/**"],
        }),
    )
    .await;
    file_cases(&app, &id, &["a"]).await;
    let case = case_ids(&app, &id).await[0].1.clone();
    for env in ["staging", "prod"] {
        app.post(
            &app.admin,
            &format!("/v1/cases/{case}/verdict"),
            json!({ "verdict": "pass", "environment": env }),
        )
        .await;
    }

    let (_, rel) = app
        .post(
            &app.admin,
            "/v1/projects/tp/releases",
            json!({ "ref": "v2", "touched_paths": ["src/billing/split.rs"] }),
        )
        .await;
    assert_eq!(
        rel["impact"]["stale_cases"], 1,
        "one case, however many environments it is verified in: {rel}"
    );
    assert_eq!(
        rel["impact"]["stale_pairs"], 2,
        "…and two units of work to redo: {rel}"
    );

    // The worklist is per (case, environment) and carries where to go, because
    // an item that does not say where is not actionable.
    let (_, wl) = app
        .get(&app.admin, "/v1/projects/tp/checklist/worklist")
        .await;
    assert_eq!(wl["agent"]["cases"], json!(2), "{wl}");
    let items = wl["agent"]["items"].as_array().unwrap();
    let slugs: Vec<&str> = items
        .iter()
        .map(|i| i["environment_slug"].as_str().unwrap())
        .collect();
    assert!(
        slugs.contains(&"staging") && slugs.contains(&"prod"),
        "{wl}"
    );
    let staging_item = items
        .iter()
        .find(|i| i["environment_slug"] == "staging")
        .unwrap();
    assert_eq!(staging_item["reason"], "stale", "{staging_item}");
    assert_eq!(
        staging_item["base_url"], "https://staging.example.com",
        "the item says where to go: {staging_item}"
    );
}

/// Dropping an environment from a check keeps what was recorded there.
///
/// It stops being required, which is not the same as never having been verified
/// there — and re-declaring it later finds the history intact rather than
/// starting from `never`.
#[tokio::test]
async fn undeclaring_an_environment_keeps_its_verdicts_for_when_it_returns() {
    let app = TestApp::spawn().await;
    environment(&app, json!({ "slug": "staging" })).await;
    environment(&app, json!({ "slug": "prod" })).await;

    let id = check(
        &app,
        json!({ "title": "Split", "environments": ["staging", "prod"] }),
    )
    .await;
    file_cases(&app, &id, &["a"]).await;
    let case = case_ids(&app, &id).await[0].1.clone();
    app.post(
        &app.admin,
        &format!("/v1/cases/{case}/verdict"),
        json!({ "verdict": "pass", "environment": "staging" }),
    )
    .await;

    // Narrow it to production only.
    let (s, patched) = app
        .patch(
            &app.admin,
            &format!("/v1/checks/{id}"),
            json!({ "environments": ["prod"] }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{patched}");
    assert_eq!(patched["environments"].as_array().unwrap().len(), 1);
    let (_, narrowed) = app.get(&app.admin, &format!("/v1/cases/{case}")).await;
    assert_eq!(narrowed["environments"].as_array().unwrap().len(), 1);
    assert_eq!(narrowed["environments"][0]["slug"], "prod");
    assert_eq!(
        narrowed["state"], "never",
        "only production counts now: {narrowed}"
    );

    // Put staging back: its verdict is still there.
    let (_, restored) = app
        .patch(
            &app.admin,
            &format!("/v1/checks/{id}"),
            json!({ "environments": ["staging", "prod"] }),
        )
        .await;
    assert_eq!(restored["environments"].as_array().unwrap().len(), 2);
    let (_, back) = app.get(&app.admin, &format!("/v1/cases/{case}")).await;
    let by_slug: std::collections::HashMap<&str, &Value> = back["environments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["slug"].as_str().unwrap(), e))
        .collect();
    assert_eq!(
        by_slug["staging"]["state"], "verified",
        "the record survived being undeclared: {back}"
    );

    // Clearing the set entirely returns the check to the unscoped reading.
    let (_, cleared) = app
        .patch(
            &app.admin,
            &format!("/v1/checks/{id}"),
            json!({ "environments": [] }),
        )
        .await;
    assert!(
        cleared["environments"].as_array().unwrap().is_empty(),
        "{cleared}"
    );
}

/// A database written before the rename opens as `checks`, with every row intact.
///
/// The rename is the one migration step that runs BEFORE the schema batch, and
/// the reason is worth pinning with a test rather than a comment: with the usual
/// ordering, `CREATE TABLE IF NOT EXISTS checks` creates an EMPTY `checks` beside
/// the populated `lanes`, the rename can then never run, and the database quietly
/// carries both — every existing row in the table nothing reads. That failure is
/// invisible on a fresh install and total on a real one.
///
/// Ids are deliberately not rewritten, so a pre-rename `lane-…` id still resolves.
#[tokio::test]
async fn a_pre_rename_database_opens_as_checks_with_its_verdicts_intact() {
    use rusqlite::params;
    use takomo::store::Store;

    // The checklist tables exactly as they were before the rename: table `lanes`,
    // table `lane_globs`, and `cases.lane`. Everything else the store needs is
    // created by the current schema batch, which is the point.
    const OLD_SCHEMA: &str = r#"
    CREATE TABLE projects (id TEXT PRIMARY KEY, name TEXT NOT NULL, workflow_json TEXT NOT NULL, created_at INTEGER NOT NULL);
    CREATE TABLE workflow_states (project TEXT NOT NULL, state TEXT NOT NULL, category TEXT NOT NULL, claimable INTEGER NOT NULL DEFAULT 0, terminal INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (project, state));
    CREATE TABLE lanes (
      id TEXT PRIMARY KEY, project TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE, epic TEXT,
      title TEXT NOT NULL, body TEXT NOT NULL DEFAULT '', precondition TEXT NOT NULL DEFAULT '',
      layer TEXT NOT NULL DEFAULT 'api', severity TEXT NOT NULL DEFAULT 'advisory', verification TEXT,
      expiry_days INTEGER, expiry_releases INTEGER, cost_agent_minutes INTEGER, cost_human_minutes INTEGER,
      metadata TEXT NOT NULL DEFAULT 'null', version INTEGER NOT NULL DEFAULT 1, created_by TEXT NOT NULL,
      created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, archived_at INTEGER);
    CREATE INDEX idx_lanes_project ON lanes(project);
    CREATE INDEX idx_lanes_epic ON lanes(epic) WHERE epic IS NOT NULL;
    CREATE TABLE lane_globs (lane TEXT NOT NULL REFERENCES lanes(id) ON DELETE CASCADE, glob TEXT NOT NULL, PRIMARY KEY (lane, glob)) WITHOUT ROWID;
    CREATE TABLE cases (
      id TEXT PRIMARY KEY, lane TEXT NOT NULL REFERENCES lanes(id) ON DELETE CASCADE, key TEXT NOT NULL,
      label TEXT NOT NULL DEFAULT '', assignment TEXT NOT NULL DEFAULT '{}', seeded INTEGER NOT NULL DEFAULT 0,
      agent_verdict TEXT, agent_at INTEGER, agent_by TEXT, agent_release TEXT,
      human_verdict TEXT, human_at INTEGER, human_by TEXT, human_release TEXT,
      stale_since TEXT, retired_at INTEGER, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
      UNIQUE(lane, key));
    CREATE INDEX idx_cases_lane ON cases(lane);
    CREATE TABLE case_verdicts (id TEXT PRIMARY KEY, case_id TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
      actor_kind TEXT NOT NULL, actor TEXT NOT NULL, verdict TEXT NOT NULL, note TEXT, release TEXT, at INTEGER NOT NULL);
    "#;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("pre-rename.db");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(OLD_SCHEMA).unwrap();
        conn.execute(
            "INSERT INTO projects (id,name,workflow_json,created_at) VALUES ('lp','Legacy','{}',1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO lanes (id,project,epic,title,body,precondition,layer,severity,metadata,version,created_by,created_at,updated_at) \
             VALUES ('lane-legacy1','lp',NULL,'Finalize an invoice','','a draft invoice','api','blocking','null',1,'seed',1,2)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO lane_globs (lane,glob) VALUES ('lane-legacy1','src/billing/**')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cases (id,lane,key,label,assignment,seeded,agent_verdict,agent_at,agent_by,created_at,updated_at) \
             VALUES ('case-legacy1','lane-legacy1','vat=19','VAT 19','{}',0,'pass',5,'agent:old',1,2)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO case_verdicts (id,case_id,actor_kind,actor,verdict,note,at) \
             VALUES (?1,'case-legacy1','agent','agent:old','pass','looked right',5)",
            params!["cv-legacy1"],
        )
        .unwrap();
    }

    // Opening with the current binary performs the rename.
    let store = Store::open(&db_path).unwrap();

    // The check is reachable by the id it has always had.
    let check = store.get_check("lane-legacy1").unwrap();
    assert_eq!(check.title, "Finalize an invoice");
    assert_eq!(check.severity, "blocking");
    assert_eq!(
        check.globs,
        vec!["src/billing/**".to_string()],
        "the glob followed its check through the rename: {:?}",
        check.globs
    );

    // The case and its verdict history survived, still reporting `verified`.
    let (case, history, _) = store.get_case("case-legacy1").unwrap();
    assert_eq!(case.check, "lane-legacy1");
    assert_eq!(case.agent_verdict.as_deref(), Some("pass"));
    assert_eq!(case.state(), "verified", "state after the rename");
    assert_eq!(history.len(), 1, "verdict history is not a casualty");
    assert_eq!(history[0].note.as_deref(), Some("looked right"));

    // And the coverage rollup counts it, which is the end-to-end proof that the
    // renamed table is the one every query now reads.
    let coverage = store.checklist_coverage("lp").unwrap();
    assert_eq!(coverage["checks"], 1, "{coverage}");
    assert_eq!(coverage["cases"]["verified"], 1, "{coverage}");

    // The old names are gone rather than shadowed, and the stale indexes with
    // them — two indexes over one column is the cost of skipping that step.
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let names: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT name FROM sqlite_master WHERE type IN ('table','index')")
                .unwrap();
            stmt.query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        for gone in ["lanes", "lane_globs", "idx_lanes_project", "idx_cases_lane"] {
            assert!(
                !names.iter().any(|n| n == gone),
                "'{gone}' should not survive the rename: {names:?}"
            );
        }
        for present in ["checks", "check_globs"] {
            assert!(
                names.iter().any(|n| n == present),
                "'{present}' should exist: {names:?}"
            );
        }
    }

    // Idempotent: a second open is a no-op, not a second rename attempt.
    drop(store);
    let store2 = Store::open(&db_path).unwrap();
    assert_eq!(
        store2.get_check("lane-legacy1").unwrap().title,
        "Finalize an invoice",
        "reopening an already-renamed database changes nothing"
    );
}

// ---------------------------------------------------------------------------
// Schedules: cadences that materialize ordinary tickets.
//
// The properties worth pinning are the ones the design rests on, not the CRUD:
// that a proposal is inert until a human acts, that one slot can only ever
// produce one ticket, that an expired occurrence stops being offered as work
// without anything transitioning it, and that occurrences never reference each
// other.
// ---------------------------------------------------------------------------

/// Materialize one live occurrence and return (schedule id, ticket id).
///
/// Goes through `/run` rather than backdating a slot, because a slot forced into
/// the past produces a ticket whose deadline is also in the past — instantly
/// `not_fulfilled`, which is the opposite of what a test wanting a live
/// occurrence needs.
async fn schedule_with_live_occurrence(app: &TestApp, name: &str) -> (String, String) {
    let (_, body) = create_schedule(app, &app.human, name).await;
    let sched = body["id"].as_str().unwrap().to_string();
    let (status, run) = app
        .post(&app.human, &format!("/v1/schedules/{sched}/run"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{run}");
    let ticket = run["ticket"]
        .as_str()
        .expect("a run creates a ticket")
        .to_string();
    (sched, ticket)
}

/// The weekly-review cadence, in the shape the API takes.
fn weekly_cadence() -> Value {
    json!({ "every": "week", "on": ["mon"], "at": "09:00", "tz": "Europe/Berlin" })
}

async fn create_schedule(app: &TestApp, token: &str, name: &str) -> (StatusCode, Value) {
    app.post(
        token,
        "/v1/schedules",
        json!({
            "project": "tp",
            "name": name,
            "cadence": weekly_cadence(),
            "template": { "title": "Weekly review — {week}", "labels": ["ritual"] },
        }),
    )
    .await
}

#[tokio::test]
async fn a_human_created_schedule_is_born_active_and_previews_its_slots() {
    let app = TestApp::spawn().await;
    let (status, body) = create_schedule(&app, &app.human, "Weekly review").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["status"], "active");
    assert!(
        body["next_slot"].is_string(),
        "an active schedule must have a next slot: {body}"
    );
    let upcoming = body["upcoming"].as_array().expect("upcoming");
    assert_eq!(
        upcoming.len(),
        3,
        "the create response previews three slots"
    );
    // Every previewed slot is 09:00 Europe/Berlin, i.e. 07:00Z in summer — the
    // whole point of computing slots in local time.
    for slot in upcoming {
        let s = slot.as_str().unwrap();
        assert!(
            s.ends_with("07:00:00.000Z") || s.ends_with("08:00:00.000Z"),
            "slot {s} should be 09:00 Berlin (07:00Z in summer, 08:00Z in winter)"
        );
    }
    // …and they are strictly increasing, a week apart.
    assert!(upcoming[0].as_str().unwrap() < upcoming[1].as_str().unwrap());
}

#[tokio::test]
async fn an_agent_proposal_is_inert_until_a_human_activates_it() {
    let app = TestApp::spawn().await;
    // `worker` carries read+write but not human — the agent case.
    let (status, body) = create_schedule(&app, &app.worker, "Rotate the deploy key").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["status"], "pending");
    assert!(
        body["next_slot"].is_null(),
        "a pending schedule must have NO next slot — that is what makes it inert \
         by construction rather than by a check: {body}"
    );
    assert_eq!(body["proposed_by"], "agent:w1");
    let msg = body["message"]
        .as_str()
        .expect("a pending create explains itself");
    assert!(
        msg.contains("NOT active") && msg.contains("Do not wait on it"),
        "the message must tell an agent not to poll: {msg}"
    );
    // It still previews what it *would* do, so a reviewer can see it.
    assert_eq!(body["upcoming"].as_array().unwrap().len(), 3);

    let id = body["id"].as_str().unwrap().to_string();

    // The sweeper cannot fire it however overdue it looks.
    app.force_schedule_slot(&id, 1);
    let created = app.open_store().materialize_due().expect("sweep");
    assert_eq!(
        created, 0,
        "a pending schedule must not fire even with a slot in the past"
    );

    // An agent cannot activate its own proposal.
    let (status, _) = app
        .post(
            &app.worker,
            &format!("/v1/schedules/{id}/activate"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // A human can.
    let (status, body) = app
        .post(
            &app.human,
            &format!("/v1/schedules/{id}/activate"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "active");
    assert!(body["next_slot"].is_string());
}

#[tokio::test]
async fn turning_the_project_flag_off_lets_a_fleet_schedule_its_own_work() {
    let app = TestApp::spawn().await;
    let (status, body) = app
        .put(
            &app.admin,
            "/v1/projects/tp/schedule-approval",
            json!({ "required": false }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["schedule_approval"], false);

    let (status, body) = create_schedule(&app, &app.worker, "Verify the backup").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(
        body["status"], "active",
        "with the flag off an agent's schedule goes live immediately"
    );

    // The flag is admin-gated: a human token cannot flip it.
    let (status, _) = app
        .put(
            &app.human,
            "/v1/projects/tp/schedule-approval",
            json!({ "required": true }),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn one_slot_can_only_ever_produce_one_ticket() {
    let app = TestApp::spawn_without_sweeper().await;
    let (_, body) = create_schedule(&app, &app.human, "Weekly review").await;
    let id = body["id"].as_str().unwrap().to_string();
    // The slot every attempt below aims at: the one the schedule would use next.
    let slot = body["next_slot"].as_str().unwrap().to_string();

    let (status, first) = app
        .post(&app.human, &format!("/v1/schedules/{id}/run"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["created"], true);

    // Rewind the schedule to that same slot and try again, twice, by both
    // routes into materialization.
    let slot_ms = chrono::DateTime::parse_from_rfc3339(&slot)
        .unwrap()
        .timestamp_millis();

    app.force_schedule_slot(&id, slot_ms);
    let (status, second) = app
        .post(&app.human, &format!("/v1/schedules/{id}/run"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(
        second["created"], false,
        "the same slot must not produce a second ticket — UNIQUE(schedule, \
         occurrence) refuses it, and that is not an error to apologise for: {second}"
    );

    app.force_schedule_slot(&id, slot_ms);
    assert_eq!(
        app.open_store().materialize_due().expect("sweep"),
        0,
        "the sweep converges on the same one ticket"
    );

    let (_, hist) = app
        .get(&app.human, &format!("/v1/schedules/{id}/occurrences"))
        .await;
    assert_eq!(
        hist["occurrences"].as_array().unwrap().len(),
        1,
        "exactly one occurrence survived three materialization attempts: {hist}"
    );
}

#[tokio::test]
async fn a_materialized_ticket_is_an_ordinary_ticket_that_links_back() {
    let app = TestApp::spawn_without_sweeper().await;
    let (id, ticket_id) = schedule_with_live_occurrence(&app, "Weekly review").await;

    let (status, t) = app
        .get(&app.human, &format!("/v1/tickets/{ticket_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{t}");
    assert_eq!(t["schedule"], id, "the ticket links back to its schedule");
    assert!(t["occurrence"].is_string(), "and carries its slot: {t}");
    assert!(t["expires_at"].is_string(), "and its deadline: {t}");
    assert_eq!(
        t["created_by"],
        format!("schedule:{id}"),
        "the event log must not claim a person filed this"
    );
    // The template's placeholder was rendered, in the cadence's own zone.
    let title = t["title"].as_str().unwrap();
    assert!(
        title.starts_with("Weekly review — 20") && title.contains("-W"),
        "{{week}} should have been substituted: {title}"
    );
    assert_eq!(t["labels"][0], "ritual");
    // It is otherwise completely ordinary: claimable through the normal queue.
    assert_eq!(t["state_category"], "todo");

    // No dependency was invented between the schedule's output and anything else.
    assert_eq!(
        t["blocked_by"].as_array().unwrap().len(),
        0,
        "an occurrence must not inherit a dependency — that is what keeps \
         occurrences independent of one another"
    );
    assert!(t["parent"].is_null());
}

#[tokio::test]
async fn an_expired_occurrence_leaves_the_ready_queue_without_being_transitioned() {
    let app = TestApp::spawn_without_sweeper().await;
    let (sched, ticket) = schedule_with_live_occurrence(&app, "Weekly review").await;
    let (_, hist) = app
        .get(&app.human, &format!("/v1/schedules/{sched}/occurrences"))
        .await;
    assert_eq!(hist["occurrences"][0]["outcome"], "open");

    // A materialized ticket lands in the workflow's *initial* state, which in
    // factory-default is `brief` and deliberately not claimable — the project's
    // own state machine decides when scheduled work becomes dispatchable. Move it
    // to a claimable state, or the assertions below would pass while testing
    // nothing.
    let (status, moved) = app.transition(&app.human, &ticket, "spec").await;
    assert_eq!(status, StatusCode::OK, "brief -> spec: {moved}");

    // While live it is offered as work.
    let (_, ready) = app.get(&app.worker, "/v1/ready?project=tp").await;
    let offered: Vec<&str> = ready["items"]
        .as_array()
        .expect("/v1/ready returns an items envelope")
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert!(
        offered.contains(&ticket.as_str()),
        "a live occurrence should be in the ready queue: {ready}"
    );

    // Now the clock runs out.
    app.force_ticket_expiry(&ticket, 2_000);

    let (_, ready) = app.get(&app.worker, "/v1/ready?project=tp").await;
    let offered: Vec<&str> = ready["items"]
        .as_array()
        .expect("/v1/ready returns an items envelope")
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert!(
        !offered.contains(&ticket.as_str()),
        "an expired occurrence must not be handed to a worker — otherwise an \
         agent keeps being given last month's review: {ready}"
    );

    // But NOTHING transitioned it. The server closes nothing; a maintenance
    // agent does, through the ordinary API.
    let (status, t) = app.get(&app.human, &format!("/v1/tickets/{ticket}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        t["state"], "spec",
        "expiry must leave the state exactly where it was — it needs no workflow \
         edge, so a project whose machine has no cancel path is unaffected"
    );
    assert!(t["archived_at"].is_null(), "not archived either");

    // It is still reachable and claimable by id — only the queue stopped
    // offering it.
    let (status, claim) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{ticket}/claim"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "still claimable by id: {claim}");

    // And the outcome now reads as not fulfilled.
    let (_, hist) = app
        .get(&app.human, &format!("/v1/schedules/{sched}/occurrences"))
        .await;
    assert_eq!(hist["occurrences"][0]["outcome"], "not_fulfilled");
}

#[tokio::test]
async fn the_expired_filter_is_how_a_maintenance_agent_finds_what_to_tidy() {
    let app = TestApp::spawn_without_sweeper().await;
    let (sched, scheduled) = schedule_with_live_occurrence(&app, "Weekly review").await;
    let handmade = app.create_ticket("An ordinary ticket").await;

    // Nothing is expired yet.
    let (_, list) = app.get(&app.human, "/v1/tickets?expired=true").await;
    assert_eq!(list["items"].as_array().unwrap().len(), 0, "{list}");

    app.force_ticket_expiry(&scheduled, 2_000);

    let (status, list) = app.get(&app.human, "/v1/tickets?expired=true").await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let ids: Vec<&str> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec![scheduled.as_str()]);

    // A hand-made ticket has no deadline, so it counts as unexpired rather than
    // being filtered out of both sides.
    let (_, list) = app.get(&app.human, "/v1/tickets?expired=false").await;
    let ids: Vec<&str> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&handmade.as_str()), "{list}");
    assert!(!ids.contains(&scheduled.as_str()));

    // Scoped to one schedule.
    let (_, list) = app
        .get(&app.human, &format!("/v1/tickets?schedule={sched}"))
        .await;
    assert_eq!(list["items"].as_array().unwrap().len(), 1, "{list}");

    let (status, err) = app.get(&app.human, "/v1/tickets?expired=maybe").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(err["code"], "validation.expired");
}

#[tokio::test]
async fn a_cron_string_is_answered_with_its_translation() {
    let app = TestApp::spawn().await;
    let (status, err) = app
        .post(
            &app.human,
            "/v1/schedules",
            json!({
                "project": "tp",
                "name": "Weekly review",
                "cadence": "0 9 * * mon",
                "template": { "title": "Weekly review" },
            }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
    assert_eq!(err["code"], "validation.schedule.cadence");
    let msg = err["message"].as_str().unwrap();
    assert!(msg.contains(r#""every":"week""#), "{msg}");
    assert!(msg.contains(r#""at":"09:00""#), "{msg}");
    assert!(
        msg.contains("tz"),
        "it must mention the missing zone: {msg}"
    );
}

#[tokio::test]
async fn a_mistyped_cadence_field_is_refused_rather_than_silently_dropped() {
    let app = TestApp::spawn().await;
    let (status, err) = app
        .post(
            &app.human,
            "/v1/schedules",
            json!({
                "project": "tp",
                "name": "Weekly review",
                // `ony` instead of `on`: accepting this would drop the weekday
                // filter and fire every single day.
                "cadence": { "every": "week", "ony": ["mon"], "at": "09:00" },
                "template": { "title": "Weekly review" },
            }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
    assert_eq!(err["code"], "validation.schedule.cadence");
    assert!(err["message"].as_str().unwrap().contains("ony"), "{err}");

    // Same for the template — parent is absent on purpose.
    let (status, err) = app
        .post(
            &app.human,
            "/v1/schedules",
            json!({
                "project": "tp",
                "name": "Weekly review",
                "cadence": weekly_cadence(),
                "template": { "title": "Weekly review", "parent": "tp-abcd" },
            }),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
    assert_eq!(err["code"], "validation.schedule.template");
}

#[tokio::test]
async fn pause_stops_it_and_resume_never_backfills_the_gap() {
    let app = TestApp::spawn_without_sweeper().await;
    let (_, body) = create_schedule(&app, &app.human, "Weekly review").await;
    let id = body["id"].as_str().unwrap().to_string();

    let (status, body) = app
        .post(&app.human, &format!("/v1/schedules/{id}/pause"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "paused");
    assert!(
        body["next_slot"].is_null(),
        "pausing must clear the slot, which is what makes it inert: {body}"
    );

    // Overdue on paper, still silent.
    app.force_schedule_slot(&id, 1_000);
    assert_eq!(
        app.open_store().materialize_due().expect("sweep"),
        0,
        "a paused schedule must not fire"
    );

    let (status, body) = app
        .post(&app.human, &format!("/v1/schedules/{id}/resume"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "active");
    let next = body["next_slot"]
        .as_str()
        .expect("a resumed schedule has a slot");
    assert!(
        next > chrono::Utc::now()
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
            .as_str(),
        "resume computes forward from now and never backfills the pause, or \
         unpausing would dump history into the ready queue: {next}"
    );
}

#[tokio::test]
async fn a_rejected_schedule_is_terminal() {
    let app = TestApp::spawn().await;
    let (_, body) = create_schedule(&app, &app.worker, "Rotate the deploy key").await;
    let id = body["id"].as_str().unwrap().to_string();

    let (status, body) = app
        .post(&app.human, &format!("/v1/schedules/{id}/reject"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "rejected");

    // It cannot be revived: a turned-down proposal is re-created, so the record
    // stays honest about what was agreed when.
    let (status, err) = app
        .post(
            &app.human,
            &format!("/v1/schedules/{id}/activate"),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{err}");
    assert_eq!(err["code"], "conflict.schedule.status");
    assert!(
        err["message"].as_str().unwrap().contains("terminal"),
        "{err}"
    );
}

#[tokio::test]
async fn deleting_a_schedule_keeps_the_work_it_produced() {
    let app = TestApp::spawn_without_sweeper().await;
    let (sched, ticket) = schedule_with_live_occurrence(&app, "Weekly review").await;

    let (status, body) = app
        .delete(&app.human, &format!("/v1/schedules/{sched}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, t) = app.get(&app.human, &format!("/v1/tickets/{ticket}")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the ticket is real work with real history and must survive the rule: {t}"
    );
    assert_eq!(
        t["schedule"], sched,
        "and keeps the dangling id as the record of where it came from — which is \
         why that column carries no foreign key"
    );

    let (status, _) = app.get(&app.human, &format!("/v1/schedules/{sched}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn patch_recomputes_the_slot_behind_an_if_match() {
    let app = TestApp::spawn().await;
    let (_, body) = create_schedule(&app, &app.human, "Weekly review").await;
    let id = body["id"].as_str().unwrap().to_string();
    let (status, _, etag) = app
        .get_raw(&app.human, &format!("/v1/schedules/{id}"))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!etag.is_empty(), "the detail view must carry an ETag");

    let before = body["next_slot"].as_str().unwrap().to_string();
    let (status, body) = app
        .patch_with(
            &app.human,
            &format!("/v1/schedules/{id}"),
            &[("If-Match", &etag)],
            json!({ "cadence": { "every": "day", "at": "06:30" } }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_ne!(
        body["next_slot"].as_str().unwrap(),
        before,
        "changing the cadence must recompute the next slot immediately"
    );

    // A stale If-Match is refused.
    let (status, err) = app
        .patch_with(
            &app.human,
            &format!("/v1/schedules/{id}"),
            &[("If-Match", "\"1\"")],
            json!({ "name": "Renamed" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{err}");
    assert_eq!(err["code"], "conflict.version");
}

#[tokio::test]
async fn downtime_fires_only_the_most_recent_slot_and_records_the_gap() {
    let app = TestApp::spawn_without_sweeper().await;
    let (_, body) = app
        .post(
            &app.human,
            "/v1/schedules",
            json!({
                "project": "tp",
                "name": "Verify the backup",
                "cadence": { "every": "day", "at": "06:30" },
                "template": { "title": "Verify the backup — {date}" },
                // Anchored well in the past, so many slots have gone by.
                "starts_at": "2026-01-01T06:30:00Z",
            }),
        )
        .await;
    let id = body["id"].as_str().unwrap().to_string();

    // Creating a schedule always looks forward — a historic `starts_at` does not
    // backfill — so stand in for downtime by rewinding the slot to the anchor,
    // which is on the cadence grid by construction.
    let anchor_ms = chrono::DateTime::parse_from_rfc3339(body["starts_at"].as_str().unwrap())
        .unwrap()
        .timestamp_millis();
    app.force_schedule_slot(&id, anchor_ms);
    let created = app.open_store().materialize_due().expect("sweep");
    assert_eq!(created, 1, "one ticket, not one per missed day");

    let (_, hist) = app
        .get(&app.human, &format!("/v1/schedules/{id}/occurrences"))
        .await;
    let occs = hist["occurrences"].as_array().unwrap();
    assert_eq!(occs.len(), 1, "{hist}");
    assert_eq!(
        occs[0]["outcome"], "open",
        "the slot it fired must be the recent one, so the ticket it made is live \
         rather than instantly stale: {hist}"
    );

    // The hole in the history is recorded rather than silent, so a gap reads as
    // "nothing was running" instead of "nothing was scheduled".
    let (_, events) = app
        .get(&app.human, "/v1/events?since=0&kind=schedule_missed")
        .await;
    let missed = events["events"].as_array().unwrap();
    assert_eq!(missed.len(), 1, "{events}");
    assert!(
        missed[0]["payload"]["slots"].as_i64().unwrap() > 1,
        "and says how many were passed over: {events}"
    );
}

#[tokio::test]
async fn the_project_cap_bounds_how_many_schedules_one_project_can_have() {
    let app = TestApp::spawn().await;
    // Below the cap this is just a loop; the point is the refusal at the top.
    for i in 0..50 {
        let (status, body) = create_schedule(&app, &app.human, &format!("S{i}")).await;
        assert_eq!(status, StatusCode::CREATED, "schedule {i}: {body}");
    }
    let (status, err) = create_schedule(&app, &app.human, "One too many").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
    assert_eq!(err["code"], "validation.schedule.too_many");
}

#[tokio::test]
async fn a_schedule_is_scoped_to_its_own_project() {
    let app = TestApp::spawn().await;
    let (_, body) = create_schedule(&app, &app.human, "Weekly review").await;
    let id = body["id"].as_str().unwrap().to_string();

    // A token allow-listed to another project cannot read it by naming the id.
    let elsewhere = app.mint("human:other", &["read", "write", "human"], Some(&["other"]));
    let (status, _) = app.get(&elsewhere, &format!("/v1/schedules/{id}")).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = app
        .post(&elsewhere, &format!("/v1/schedules/{id}/pause"), json!({}))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_list_orders_waiting_for_you_first() {
    let app = TestApp::spawn().await;
    create_schedule(&app, &app.human, "Active one").await;
    create_schedule(&app, &app.worker, "Proposed one").await;
    let (_, body) = create_schedule(&app, &app.human, "Paused one").await;
    let paused = body["id"].as_str().unwrap().to_string();
    app.post(
        &app.human,
        &format!("/v1/schedules/{paused}/pause"),
        json!({}),
    )
    .await;

    let (status, body) = app.get(&app.human, "/v1/schedules?project=tp").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let statuses: Vec<&str> = body["schedules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["status"].as_str().unwrap())
        .collect();
    assert_eq!(
        statuses,
        vec!["pending", "active", "paused"],
        "the page renders in this order, so the API returns it in this order: {body}"
    );

    let (_, body) = app
        .get(&app.human, "/v1/schedules?project=tp&status=pending")
        .await;
    assert_eq!(body["schedules"].as_array().unwrap().len(), 1);

    let (status, err) = app
        .get(&app.human, "/v1/schedules?project=tp&status=nonsense")
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(err["code"], "validation.schedule.status");
}

#[tokio::test]
async fn a_finished_occurrence_reads_as_done_however_late() {
    let app = TestApp::spawn_without_sweeper().await;
    let (sched, ticket) = schedule_with_live_occurrence(&app, "Weekly review").await;

    // Expire it first, then finish it: terminal state must win.
    app.force_ticket_expiry(&ticket, 2_000);
    let (status, body) = app.transition(&app.human, &ticket, "cancelled").await;
    assert_eq!(status, StatusCode::OK, "brief -> cancelled: {body}");

    let (_, hist) = app
        .get(&app.human, &format!("/v1/schedules/{sched}/occurrences"))
        .await;
    assert_eq!(
        hist["occurrences"][0]["outcome"], "done",
        "a ticket somebody finished is done however late — terminal state wins \
         over the clock: {hist}"
    );
}

/// `/schedules` is served like the other pages, with the same defence-in-depth
/// headers — it holds a bearer token in localStorage exactly as they do.
///
/// PORTED (phase 2 of 4): the renderer is a bundled module now, so asserting on
/// `var MD_INLINE` would only assert on minifier output. Its behaviour — which
/// is what makes a proposal's ticket body readable to whoever approves it — is
/// covered by 30 tests in web/src/lib/markdown.test.ts. What remains this
/// layer's job, and is checked here, is that the served document is the
/// self-contained build: one document, no second request.
#[tokio::test]
async fn schedules_page_is_served_as_a_self_contained_build() {
    let app = TestApp::spawn().await;
    let resp = app.request(Method::GET, "/schedules").send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let headers = resp.headers().clone();
    assert_eq!(headers["x-frame-options"], "DENY");
    assert!(headers["content-security-policy"]
        .to_str()
        .unwrap()
        .contains("frame-ancestors 'none'"));
    assert_app_shell("/schedules", &resp.text().await.unwrap());

    // The page exists to show a cadence and its history, and to let a human act
    // on a proposal. All three vocabularies must ship.
    let bundle = app.eager_bundle().await;
    assert!(bundle.contains("/schedules"), "no schedules fetches");
    assert!(bundle.contains("occurrences"), "no occurrence history");
    assert!(bundle.contains("activate"), "no activate action");
}

/// Every page links to every other one, so the four surfaces read as one product
/// rather than four apps that happen to share a palette.
///
/// The links are built in JS and the minifier picks the quoting — it currently
/// emits backticks (``board:`/board` ``) — so asserting on quotes would be
/// asserting on minifier output. The stable signal is the path itself.
#[tokio::test]
async fn every_spa_links_to_the_schedules_page() {
    let app = TestApp::spawn().await;
    let bundle = app.eager_bundle().await;
    // One nav rail, one bundle — so this is asserted once rather than per page.
    // Every surface mounts that same rail, which is what makes the five read as
    // one product instead of five apps sharing a palette.
    for href in [
        "/board",
        "/documents",
        "/inbox",
        "/initiatives",
        "/schedules",
        "/settings",
        "/verification",
        "/environments",
    ] {
        assert!(
            bundle.contains(href),
            "the shared header has no link to {href} — a reader would have to know the URL"
        );
    }
}

/// The two verification surfaces are served, and their clients ship in the bundle.
///
/// The page markup is one shared shell on every route, so asserting on the HTML
/// would assert on something identical everywhere. What this layer can still
/// prove is that each surface's vocabulary reached the bundle at all — a page
/// whose API client got tree-shaken away serves a 200 and does nothing.
#[tokio::test]
async fn the_verification_surfaces_are_served_and_carry_their_clients() {
    let app = TestApp::spawn().await;

    for path in ["/verification", "/environments"] {
        let resp = app.request(Method::GET, path).send().await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "{path} should serve");
        let headers = resp.headers().clone();
        // The same defense-in-depth as every page route: these hold a bearer
        // token in localStorage.
        assert_eq!(headers["x-frame-options"], "DENY", "{path}");
        assert!(headers["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("frame-ancestors 'none'"));
        assert_app_shell(path, &resp.text().await.unwrap());
    }

    let bundle = app.eager_bundle().await;
    for fragment in [
        "/checklist/worklist",
        "/checklist/gate",
        "/environments",
        "/verdict",
    ] {
        assert!(
            bundle.contains(fragment),
            "the verification client is missing `{fragment}` — the page would render empty"
        );
    }
}

/// The seeded demo carries the two schedules the page needs to be worth opening:
/// one active with history, and one an agent proposed and left pending.
///
/// Both are anchored to a fixed past instant rather than "now minus N days", so
/// the fixture is deterministic whatever day the template is baked on.
#[tokio::test]
async fn the_dev_seed_ships_schedules_worth_looking_at() {
    let app = TestApp::spawn().await;
    let store = app.open_store();
    takomo::seed::dev(&store).expect("seed");

    let rows = store
        .list_schedules(
            &takomo::store::ScheduleListFilter {
                project: Some("demo".to_string()),
                status: None,
                allowed_projects: None,
            },
            50,
        )
        .expect("list");
    assert_eq!(rows.len(), 2, "the dev preset seeds two schedules");

    // Waiting-for-you first, which is the order the page renders.
    assert_eq!(rows[0].status, "pending");
    assert!(
        rows[0].proposed_by.is_some() && rows[0].rationale.is_some(),
        "a proposal needs a proposer and a rationale, or the confirm row has \
         nothing for a reviewer to judge: {:?}",
        rows[0]
    );
    assert!(
        rows[0].next_slot.is_none(),
        "a pending schedule must have no next slot — that is what makes it inert"
    );

    assert_eq!(rows[1].status, "active");
    assert!(rows[1].next_slot.is_some());

    // The active one has a history, and a MIXED one — the strip's whole value is
    // showing a cadence that is being kept unevenly, so a fixture where every
    // cell is the same colour would demo nothing.
    let occ = store
        .schedule_occurrences(&rows[1].id, 8)
        .expect("occurrences");
    assert!(
        occ.len() >= 5,
        "the seeded strip needs history to be worth opening, got {} occurrence(s)",
        occ.len()
    );
    let done = occ.iter().filter(|o| o.outcome == "done").count();
    let nf = occ.iter().filter(|o| o.outcome == "not_fulfilled").count();
    assert!(
        done > 0 && nf > 0,
        "the strip should show both kept and missed occurrences, got {done} done / {nf} not \
         fulfilled: {occ:?}"
    );
    // Built through the real firing path, so every occurrence is a real ticket on
    // a real cadence slot — not a row the seeder drew to look like one.
    for o in &occ {
        assert!(
            o.ticket.starts_with("demo-"),
            "occurrence {o:?} should point at a real demo ticket"
        );
        assert!(
            o.expires_at.is_some(),
            "every occurrence carries a deadline"
        );
    }
}

/// The board card says where a scheduled ticket came from, and says when its
/// clock has run out — the only place a reader learns that, since expiry
/// transitions nothing.
///
/// PORTED: asserted against the bytes the binary serves, on the vocabulary that
/// survives the build. The old page was checked for `L().fromSchedule` and the
/// `\u21bb` escape; a bundled page carries the ↻ character itself and reaches
/// its strings through a compiled table, so those spellings say nothing now.
#[tokio::test]
async fn the_board_marks_scheduled_and_not_fulfilled_cards() {
    let app = TestApp::spawn().await;
    let page = app.eager_bundle().await;
    assert!(
        page.contains("fromSchedule") && page.contains("\u{21bb}"),
        "the board should carry the ↻ provenance chip"
    );
    assert!(
        page.contains("notFulfilled") && page.contains("expires_at"),
        "the board should mark an occurrence whose deadline passed"
    );
    // Both locales, still: a key in only one renders as `undefined` for whoever
    // gets the other. The web build makes this a compile error too, but the
    // served bytes are what a reader actually gets.
    for key in ["fromSchedule", "notFulfilled", "schedules"] {
        assert!(
            page.matches(key).count() >= 2,
            "`{key}` must reach the served page in both the de and en tables"
        );
    }
}

// ---------------------------------------------------------------------------
// Bounded reads: every list says how much it left out
// ---------------------------------------------------------------------------

/// `/v1/ready` answers with an envelope carrying `total`, not the bare array it
/// used to return.
///
/// The array had nowhere to report what it omitted, so a caller that asked for
/// 20 and got 20 could not tell a queue of 20 from the head of a queue of 137 —
/// and an agent draining work would read a fraction of it as the whole thing.
#[tokio::test]
async fn ready_reports_the_whole_queue_alongside_the_page() {
    let app = TestApp::spawn().await;
    for i in 0..4 {
        let id = app.create_ticket(&format!("ready {i}")).await;
        app.to_ready(&id).await;
    }

    let (status, all) = app.get(&app.admin, "/v1/ready?project=tp").await;
    assert_eq!(status, StatusCode::OK, "{all}");
    let n = all["items"].as_array().expect("items envelope").len() as i64;
    assert_eq!(n, 4, "{all}");
    assert_eq!(all["total"], 4);
    assert_eq!(all["limit"], 20, "the documented default page size");
    assert!(
        all["note"].is_null(),
        "a page holding the whole queue must not claim otherwise: {all}"
    );

    // Clipped: total still counts the queue, and the note says what to do.
    let (_, page) = app.get(&app.admin, "/v1/ready?project=tp&limit=2").await;
    assert_eq!(page["items"].as_array().unwrap().len(), 2);
    assert_eq!(page["total"], 4, "total is the queue, not the page: {page}");
    assert_eq!(page["limit"], 2);
    let note = page["note"]
        .as_str()
        .expect("a clipped page explains itself");
    assert!(
        note.contains("limit"),
        "the note should name the way to see more: {note}"
    );

    // Clamped at both ends rather than refused, so a caller cannot ask for a
    // page large enough to be a denial of service against itself.
    let (_, big) = app
        .get(&app.admin, "/v1/ready?project=tp&limit=100000")
        .await;
    assert_eq!(big["limit"], 200);
    let (_, small) = app.get(&app.admin, "/v1/ready?project=tp&limit=0").await;
    assert_eq!(small["limit"], 1);

    // `total` counts with the same predicate that selects the page: a blocked
    // ticket is absent from both, not from one.
    let blocker = app.create_ticket("blocker").await;
    let blocked = app.create_ticket("blocked").await;
    app.to_ready(&blocked).await;
    let (s, b) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{blocked}/deps"),
            json!({ "blocked_by": blocker }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let (_, after) = app.get(&app.admin, "/v1/ready?project=tp").await;
    let ids: Vec<&str> = after["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert!(!ids.contains(&blocked.as_str()), "{after}");
    assert_eq!(
        after["total"], 4,
        "a blocked ticket must not be counted by a total the page excludes it from: {after}"
    );
}

/// Checks and cases are bounded, and report the count they were bounded from.
///
/// Cases are the sharp case: they are *generated*, so one PICT model can land
/// thousands under a single check — all of which this used to return in one
/// reply.
#[tokio::test]
async fn check_and_case_lists_are_bounded_and_say_so() {
    let app = TestApp::spawn().await;
    let check_id = check(
        &app,
        json!({ "title": "checkout", "layer": "api", "severity": "blocking" }),
    )
    .await;
    let keys: Vec<String> = (0..7).map(|i| format!("k{i}")).collect();
    let key_refs: Vec<&str> = keys.iter().map(String::as_str).collect();
    file_cases(&app, &check_id, &key_refs).await;

    // Whole set: total equals what came back, and nothing claims to be partial.
    let (status, all) = app
        .get(&app.admin, &format!("/v1/checks/{check_id}/cases"))
        .await;
    assert_eq!(status, StatusCode::OK, "{all}");
    assert_eq!(all["items"].as_array().unwrap().len(), 7);
    assert_eq!(all["total"], 7);
    assert!(all["note"].is_null(), "{all}");

    // A page, and the offset that reads the next one. Cases order by `key`, so
    // the pages are stable and must not overlap.
    let (_, first) = app
        .get(&app.admin, &format!("/v1/checks/{check_id}/cases?limit=3"))
        .await;
    assert_eq!(first["items"].as_array().unwrap().len(), 3);
    assert_eq!(first["total"], 7, "total ignores the page size: {first}");
    assert_eq!(first["offset"], 0);
    assert!(
        first["note"]
            .as_str()
            .unwrap_or_default()
            .contains("offset=3"),
        "the note should hand back the next offset: {first}"
    );

    let (_, second) = app
        .get(
            &app.admin,
            &format!("/v1/checks/{check_id}/cases?limit=3&offset=3"),
        )
        .await;
    let page1: Vec<&str> = first["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["key"].as_str().unwrap())
        .collect();
    let page2: Vec<&str> = second["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["key"].as_str().unwrap())
        .collect();
    assert_eq!(page1, ["k0", "k1", "k2"], "ordered by key");
    assert_eq!(
        page2,
        ["k3", "k4", "k5"],
        "the next page, disjoint from the first"
    );

    // Checks: same envelope.
    for i in 0..3 {
        check(
            &app,
            json!({ "title": format!("check {i}"), "layer": "ui", "severity": "advisory" }),
        )
        .await;
    }
    let (_, checks) = app.get(&app.admin, "/v1/projects/tp/checks?limit=2").await;
    assert_eq!(checks["items"].as_array().unwrap().len(), 2);
    assert_eq!(
        checks["total"], 4,
        "one blocking check plus three advisory: {checks}"
    );
    assert!(checks["note"].is_string(), "{checks}");

    // The page size applies AFTER the severity filter, not as a SQL LIMIT before
    // it — otherwise narrowing would return a page short for a reason the caller
    // cannot see.
    let (_, filtered) = app
        .get(
            &app.admin,
            "/v1/projects/tp/checks?severity=advisory&limit=3",
        )
        .await;
    assert_eq!(filtered["items"].as_array().unwrap().len(), 3);
    assert_eq!(
        filtered["total"], 3,
        "total counts what matched the filter: {filtered}"
    );
    assert!(filtered["note"].is_null(), "{filtered}");
}

/// A dependency walk reports whether it was cut short. The flag is always
/// present, so a reader never has to infer completeness from its absence.
#[tokio::test]
async fn a_dependency_graph_states_whether_it_is_complete() {
    let app = TestApp::spawn().await;
    let a = app.create_ticket("a").await;
    let b = app.create_ticket("b").await;
    let (s, body) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{a}/deps"),
            json!({ "blocked_by": b }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");

    let (status, graph) = app
        .get(&app.admin, &format!("/v1/tickets/{a}/deps?transitive=true"))
        .await;
    assert_eq!(status, StatusCode::OK, "{graph}");
    assert_eq!(
        graph["truncated"], false,
        "a small graph is complete, and says so rather than staying silent: {graph}"
    );
    assert!(
        graph["note"].is_null(),
        "a complete graph carries no partial-graph warning: {graph}"
    );
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 2, "{graph}");
}

/// The question inbox reports its true size, and its cursor is derived from that
/// rather than guessed from a full page.
///
/// The old rule was "this page is full, so there is probably more", which hands
/// back a cursor to an empty page whenever the queue is an exact multiple of the
/// page size — the caller then makes a request to learn nothing.
#[tokio::test]
async fn the_question_inbox_pages_on_a_real_count() {
    let app = TestApp::spawn().await;
    for i in 0..4 {
        let t = app.create_ticket(&format!("q host {i}")).await;
        app.ask(
            &app.worker,
            json!({ "ticket": t, "mode": "advisory", "kind": "confirm", "title": format!("question {i}") }),
        )
        .await;
    }

    let (status, all) = app.get(&app.human, "/v1/questions?project=tp").await;
    assert_eq!(status, StatusCode::OK, "{all}");
    assert_eq!(all["items"].as_array().unwrap().len(), 4);
    assert_eq!(all["total"], 4);
    assert!(all["next_cursor"].is_null(), "nothing left to fetch: {all}");
    assert!(all["note"].is_null(), "{all}");

    // A partial page: total counts the queue, and the cursor advances.
    let (_, first) = app
        .get(&app.human, "/v1/questions?project=tp&limit=3")
        .await;
    assert_eq!(first["items"].as_array().unwrap().len(), 3);
    assert_eq!(first["total"], 4);
    assert_eq!(first["next_cursor"], 3);
    assert!(first["note"].is_string(), "{first}");

    let (_, second) = app
        .get(&app.human, "/v1/questions?project=tp&limit=3&cursor=3")
        .await;
    assert_eq!(second["items"].as_array().unwrap().len(), 1);
    assert_eq!(second["total"], 4);
    assert!(
        second["next_cursor"].is_null(),
        "the last page must not point at an empty one: {second}"
    );

    // The boundary the old heuristic got wrong: a page that exactly exhausts the
    // queue is the end, and must not offer a cursor.
    let (_, exact) = app
        .get(&app.human, "/v1/questions?project=tp&limit=4")
        .await;
    assert_eq!(exact["items"].as_array().unwrap().len(), 4);
    assert!(
        exact["next_cursor"].is_null(),
        "a full page that is also the whole queue is not 'probably more': {exact}"
    );
}

// ---------------------------------------------------------------------------
// Moving tickets between projects (POST /v1/tickets/move).
// ---------------------------------------------------------------------------

/// A second project whose workflow deliberately does NOT define `spec`, so the
/// state-reset path is reachable, and whose initial state is `ready`.
async fn beta_project(app: &TestApp) {
    let (s, b) = app
        .post(
            &app.admin,
            "/v1/projects",
            json!({
                "id": "beta",
                "name": "Beta",
                "workflow": {
                    "name": "beta-wf",
                    "initial": "ready",
                    "states": [
                        { "id": "ready", "category": "todo", "claimable": true },
                        { "id": "review", "category": "review" },
                        { "id": "done", "category": "done", "terminal": true },
                        { "id": "cancelled", "category": "cancelled", "terminal": true }
                    ],
                    "transitions": [
                        { "from": "ready", "to": "review" },
                        { "from": "review", "to": "done" },
                        { "from": "ready", "to": "cancelled" }
                    ]
                }
            }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
}

/// The headline case: an epic moves with everything beneath it, and every id
/// survives — a moved ticket is still findable by the id a commit message quotes.
#[tokio::test]
async fn moving_an_epic_takes_its_whole_subtree_and_keeps_every_id() {
    let app = TestApp::spawn().await;
    beta_project(&app).await;

    let epic = app.create_typed("Billing", "epic", None).await;
    let child = app.create_typed("Invoices", "task", Some(&epic)).await;
    let grandchild = app.create_typed("PDF render", "task", Some(&child)).await;

    let (s, out) = app
        .post(
            &app.admin,
            "/v1/tickets/move",
            json!({ "tickets": [epic], "to_project": "beta" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{out}");
    assert_eq!(out["total"], 3, "epic + child + grandchild: {out}");
    assert!(
        out["orphaned"].as_array().unwrap().is_empty(),
        "nothing is orphaned when the subtree comes along: {out}"
    );

    for id in [&epic, &child, &grandchild] {
        let (s, t) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
        assert_eq!(
            s,
            StatusCode::OK,
            "the id still resolves after the move: {t}"
        );
        assert_eq!(t["project"], "beta", "{t}");
        assert_eq!(t["id"], id.as_str(), "a move never rewrites an id");
        assert!(
            id.starts_with("tp-"),
            "the id prefix is where it was filed, not where it lives: {id}"
        );
    }
    // The tree is intact: only its project changed.
    let (_, c) = app.get(&app.admin, &format!("/v1/tickets/{child}")).await;
    assert_eq!(c["parent"], epic.as_str(), "{c}");
    assert_eq!(c["parent_cleared"], Value::Null, "not a ticket field: {c}");
    let (_, g) = app
        .get(&app.admin, &format!("/v1/tickets/{grandchild}"))
        .await;
    assert_eq!(g["parent"], child.as_str(), "{g}");
}

/// `descendants: false` is the orphaning variant the caller asks for explicitly:
/// the epic goes, its children stay, and the response names every one of them
/// rather than leaving the caller to discover it.
#[tokio::test]
async fn moving_without_descendants_orphans_the_children_and_says_so() {
    let app = TestApp::spawn().await;
    beta_project(&app).await;

    let epic = app.create_typed("Billing", "epic", None).await;
    let child = app.create_typed("Invoices", "task", Some(&epic)).await;

    let (s, out) = app
        .post(
            &app.admin,
            "/v1/tickets/move",
            json!({ "tickets": [epic], "to_project": "beta", "descendants": false }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{out}");
    assert_eq!(out["total"], 1, "only the named ticket moved: {out}");
    assert_eq!(out["orphaned"], json!([child]), "{out}");
    assert!(
        out["note"].as_str().unwrap_or("").contains("parent"),
        "the response explains the orphaning in prose: {out}"
    );

    let (_, c) = app.get(&app.admin, &format!("/v1/tickets/{child}")).await;
    assert_eq!(c["project"], "tp", "the child stayed behind: {c}");
    assert_eq!(
        c["parent"],
        Value::Null,
        "a parent and child cannot straddle two projects: {c}"
    );
}

/// A state the target workflow does not define cannot be kept — nothing could
/// transition out of it. It lands on the target's initial state, and the caller
/// is told, per ticket.
#[tokio::test]
async fn a_state_the_target_workflow_lacks_lands_on_its_initial_state() {
    let app = TestApp::spawn().await;
    beta_project(&app).await;

    let specced = app.create_ticket("in spec").await;
    let (s, b) = app.transition(&app.human, &specced, "spec").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    // `ready` exists in both workflows, so it must survive the move untouched.
    let kept = app.create_ticket("already ready").await;
    app.to_ready(&kept).await;

    let (s, out) = app
        .post(
            &app.admin,
            "/v1/tickets/move",
            json!({ "tickets": [specced, kept], "to_project": "beta" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{out}");
    let moved = out["moved"].as_array().unwrap();
    let reset = &moved[0];
    assert_eq!(reset["from_state"], "spec", "{reset}");
    assert_eq!(reset["to_state"], "ready", "beta's initial state: {reset}");
    assert_eq!(reset["state_reset"], true, "{reset}");
    let survived = &moved[1];
    assert_eq!(survived["to_state"], "ready", "{survived}");
    assert_eq!(
        survived["state_reset"], false,
        "a state both workflows define is kept, not reset: {survived}"
    );
    assert!(
        out["note"].as_str().unwrap_or("").contains("initial"),
        "a state reset is never silent: {out}"
    );

    let (_, t) = app.get(&app.admin, &format!("/v1/tickets/{specced}")).await;
    assert_eq!(t["state"], "ready", "{t}");
    assert_eq!(t["state_category"], "todo", "{t}");
}

/// A lease is held against the workflow the move is about to change, so the
/// whole call is refused rather than splitting a subtree across two projects.
#[tokio::test]
async fn a_claimed_ticket_refuses_the_whole_move() {
    let app = TestApp::spawn().await;
    beta_project(&app).await;

    let epic = app.create_typed("Billing", "epic", None).await;
    let child = app.create_typed("Invoices", "task", Some(&epic)).await;
    app.to_ready(&child).await;
    app.claim(&child).await;

    let (s, err) = app
        .post(
            &app.admin,
            "/v1/tickets/move",
            json!({ "tickets": [epic], "to_project": "beta" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{err}");
    assert_eq!(err["code"], "claim.held", "{err}");
    assert_eq!(
        err["details"]["claimed"][0]["ticket"],
        child.as_str(),
        "{err}"
    );

    // Nothing moved: the refusal is the whole transaction, not a partial one.
    for id in [&epic, &child] {
        let (_, t) = app.get(&app.admin, &format!("/v1/tickets/{id}")).await;
        assert_eq!(t["project"], "tp", "{t}");
    }
}

/// A move crosses a token's project boundary twice, so both ends are checked —
/// a token that may write in the source cannot use a move to reach a project it
/// was never given.
#[tokio::test]
async fn a_move_is_refused_at_either_end_of_a_scoped_token() {
    let app = TestApp::spawn().await;
    beta_project(&app).await;
    let ticket = app.create_ticket("scoped").await;

    let tp_only = app.mint("agent:tp", &["read", "write"], Some(&["tp"]));
    let (s, err) = app
        .post(
            &tp_only,
            "/v1/tickets/move",
            json!({ "tickets": [ticket], "to_project": "beta" }),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "the target is out of scope: {err}"
    );

    let beta_only = app.mint("agent:beta", &["read", "write"], Some(&["beta"]));
    let (s, err) = app
        .post(
            &beta_only,
            "/v1/tickets/move",
            json!({ "tickets": [ticket], "to_project": "beta" }),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "the source is out of scope: {err}"
    );

    let (_, t) = app.get(&app.admin, &format!("/v1/tickets/{ticket}")).await;
    assert_eq!(t["project"], "tp", "neither refusal moved anything: {t}");
}

/// The things keyed on a project that have to follow the ticket: its tags are
/// registered in the target project, and its questions are filed there.
#[tokio::test]
async fn a_move_carries_the_tickets_tags_and_questions_into_the_target() {
    let app = TestApp::spawn().await;
    beta_project(&app).await;

    let (s, t) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "tp", "title": "tagged", "tags": ["component:billing"] }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let id = t["id"].as_str().unwrap().to_string();

    let (s, q) = app
        .post(
            &app.worker,
            "/v1/questions",
            json!({
                "ticket": id,
                "kind": "confirm",
                "title": "Ship it?",
                "mode": "advisory"
            }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{q}");

    let (s, out) = app
        .post(
            &app.admin,
            "/v1/tickets/move",
            json!({ "tickets": [id], "to_project": "beta" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{out}");

    let (_, tags) = app.get(&app.admin, "/v1/projects/beta/tags").await;
    let handles: Vec<&str> = tags["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["handle"].as_str().unwrap())
        .collect();
    assert!(
        handles.contains(&"billing"),
        "a tag reference is into the project's registry, so the target gets it: {tags}"
    );

    let (_, in_beta) = app.get(&app.human, "/v1/questions?project=beta").await;
    assert_eq!(
        in_beta["total"], 1,
        "the question followed its ticket: {in_beta}"
    );
    let (_, in_tp) = app.get(&app.human, "/v1/questions?project=tp").await;
    assert_eq!(in_tp["total"], 0, "and left the old project: {in_tp}");
}

/// Moving into the project a ticket is already in is a no-op that says so, and
/// the request shape is validated with the same teaching errors as everything
/// else on this surface.
#[tokio::test]
async fn a_move_validates_its_request_and_reports_no_op_tickets() {
    let app = TestApp::spawn().await;
    beta_project(&app).await;
    let id = app.create_ticket("staying put").await;

    let (s, out) = app
        .post(
            &app.admin,
            "/v1/tickets/move",
            json!({ "tickets": [id], "to_project": "tp" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{out}");
    assert_eq!(out["total"], 0, "{out}");
    assert_eq!(out["unchanged"], json!([id]), "{out}");

    let (s, err) = app
        .post(
            &app.admin,
            "/v1/tickets/move",
            json!({ "tickets": [], "to_project": "beta" }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{err}");

    let (s, err) = app
        .post(
            &app.admin,
            "/v1/tickets/move",
            json!({ "tickets": [id], "to_project": "beta", "descendants": "yes" }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
    assert_eq!(err["code"], "validation.descendants", "{err}");

    let (s, err) = app
        .post(
            &app.admin,
            "/v1/tickets/move",
            json!({ "tickets": [id], "to_project": "nope" }),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "an unknown target project: {err}");

    let (s, err) = app
        .post(
            &app.admin,
            "/v1/tickets/move",
            json!({ "tickets": [id], "to_project": "beta", "recursive": true }),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{err}");
    assert_eq!(err["code"], "validation.unknown_field", "{err}");
}

/// The move is on the event log, with what it had to reconcile, so a reader that
/// finds a ticket in a project its history never mentions can see why.
#[tokio::test]
async fn a_move_emits_an_event_carrying_what_it_reconciled() {
    let app = TestApp::spawn().await;
    beta_project(&app).await;
    let id = app.create_ticket("moved").await;
    let (s, b) = app.transition(&app.human, &id, "spec").await;
    assert_eq!(s, StatusCode::OK, "{b}");

    let (s, out) = app
        .post(
            &app.admin,
            "/v1/tickets/move",
            json!({ "tickets": [id], "to_project": "beta" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{out}");

    let (s, events) = app.get(&app.admin, "/v1/events?since=0&project=beta").await;
    assert_eq!(s, StatusCode::OK, "{events}");
    let moved = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["kind"] == "ticket_moved")
        .unwrap_or_else(|| panic!("a ticket_moved event: {events}"));
    assert_eq!(moved["ticket"], id.as_str(), "{moved}");
    assert_eq!(moved["payload"]["from_project"], "tp", "{moved}");
    assert_eq!(moved["payload"]["to_project"], "beta", "{moved}");
    assert_eq!(moved["payload"]["state_reset"], true, "{moved}");
}

// ---------------------------------------------------------------------------
// Workflow library + dry-run validation + layout (Phase 2 of workflow config).

/// The dry-run answers the same question the PUT does, and writes nothing.
///
/// The editor validates a draft while it is being typed, so it cannot be a
/// write. What makes it trustworthy is that it runs the SAME `validate` against
/// the SAME live in-use states — if it drifted, the editor would call a draft
/// clean and the Apply a moment later would 422.
#[tokio::test]
async fn workflow_dry_run_validates_without_writing() {
    let app = TestApp::spawn().await;
    let (_, before) = app.get(&app.admin, "/v1/projects/tp/workflow").await;

    // A structurally valid workflow validates clean.
    let good = json!({
        "name": "two-state",
        "initial": "open",
        "states": [
            { "id": "open", "category": "todo", "claimable": true },
            { "id": "done", "category": "done", "terminal": true }
        ],
        "transitions": [{ "from": "open", "to": "done" }]
    });
    let (s, body) = app
        .post(
            &app.admin,
            "/v1/projects/tp/workflow/validate",
            good.clone(),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["valid"], true, "{body}");
    assert_eq!(body["problems"].as_array().unwrap().len(), 0);

    // An invalid one reports problems as data rather than throwing an error the
    // editor would have to parse out of a sentence.
    let bad = json!({
        "name": "no-terminal",
        "initial": "open",
        "states": [{ "id": "open", "category": "todo" }],
        "transitions": []
    });
    let (s, body) = app
        .post(&app.admin, "/v1/projects/tp/workflow/validate", bad)
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["valid"], false, "{body}");
    assert!(
        body["problems"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p.as_str().unwrap_or_default().contains("terminal")),
        "problems name the missing terminal state: {body}"
    );

    // And the project's workflow is untouched by either call.
    let (_, after) = app.get(&app.admin, "/v1/projects/tp/workflow").await;
    assert_eq!(before, after, "dry-run must not write");
}

/// The dry-run sees stranded tickets, which is the one rule a browser could
/// never compute for itself: it depends on where this project's tickets
/// currently sit, not on the document.
#[tokio::test]
async fn workflow_dry_run_reports_stranded_tickets() {
    let app = TestApp::spawn().await;
    // The seeded project's workflow has `todo`; put a ticket in it, then offer a
    // workflow that does not define `todo` at all.
    let id = app.create_ticket("Stranded by the new workflow").await;
    assert!(!id.is_empty());

    let without_todo = json!({
        "name": "no-todo",
        "initial": "start",
        "states": [
            { "id": "start", "category": "todo", "claimable": true },
            { "id": "finished", "category": "done", "terminal": true }
        ],
        "transitions": [{ "from": "start", "to": "finished" }]
    });
    let (s, body) = app
        .post(
            &app.admin,
            "/v1/projects/tp/workflow/validate",
            without_todo.clone(),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["valid"], false, "{body}");
    let problems = body["problems"].as_array().unwrap();
    assert!(
        problems
            .iter()
            .any(|p| p.as_str().unwrap_or_default().contains("no longer defines")),
        "the stranding problem is reported: {body}"
    );

    // The PUT refuses the same document, so preflight and apply agree.
    let (s, body) = app
        .put(&app.admin, "/v1/projects/tp/workflow", without_todo)
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "workflow.invalid");
}

/// The library ships the two built-in workflows and refuses to let them be
/// edited — they are reseeded on every start, so an edit here would be undone
/// silently at the next restart.
#[tokio::test]
async fn workflow_library_ships_builtins_and_protects_them() {
    let app = TestApp::spawn().await;

    let (s, list) = app.get(&app.admin, "/v1/workflows").await;
    assert_eq!(s, StatusCode::OK, "{list}");
    let rows = list.as_array().expect("array");
    let names: Vec<&str> = rows.iter().filter_map(|r| r["name"].as_str()).collect();
    assert!(
        names.contains(&"factory-default") && names.contains(&"simple"),
        "both shipped workflows are in the library: {names:?}"
    );
    // `simple` was NOT reachable from the server before this: the only copy the
    // process had was inside the CLI shell script.
    let simple = rows.iter().find(|r| r["name"] == "simple").expect("simple");
    assert_eq!(simple["builtin"], true);
    assert_eq!(simple["workflow"]["initial"], "todo");

    let id = simple["id"].as_str().unwrap();
    let (s, body) = app
        .patch(
            &app.admin,
            &format!("/v1/workflows/{id}"),
            json!({ "description": "mine" }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "workflow.builtin", "{body}");

    let (s, _) = app.delete(&app.admin, &format!("/v1/workflows/{id}")).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);

    // A user entry may not squat a built-in's name either — the seed would
    // overwrite it on the next start.
    let (s, body) = app
        .post(
            &app.admin,
            "/v1/workflows",
            json!({ "name": "simple", "workflow": simple["workflow"].clone() }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "workflow.name_reserved", "{body}");
}

/// A user entry round-trips, validates on the way in, and its name is unique.
#[tokio::test]
async fn workflow_library_crud_validates_and_keeps_names_unique() {
    let app = TestApp::spawn().await;
    let doc = json!({
        "name": "lean",
        "initial": "open",
        "states": [
            { "id": "open", "category": "todo", "claimable": true },
            { "id": "shipped", "category": "done", "terminal": true }
        ],
        "transitions": [{ "from": "open", "to": "shipped", "requires": ["claim"] }]
    });

    let (s, created) = app
        .post(
            &app.admin,
            "/v1/workflows",
            json!({ "name": "Lean", "description": "two states", "workflow": doc.clone(),
                    "layout": { "open": { "x": 0, "y": 0 } } }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{created}");
    assert_eq!(created["builtin"], false);
    assert_eq!(created["layout"]["open"]["x"], 0);
    let id = created["id"].as_str().unwrap().to_string();

    // A second entry cannot take the same name.
    let (s, body) = app
        .post(
            &app.admin,
            "/v1/workflows",
            json!({ "name": "Lean", "workflow": doc }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "workflow.name_taken", "{body}");

    // An invalid document is refused with the same structured problems the
    // project PUT returns, so one editor can render either.
    let (s, body) = app
        .post(
            &app.admin,
            "/v1/workflows",
            json!({
                "name": "broken",
                "workflow": {
                    "name": "broken", "initial": "a",
                    "states": [{ "id": "a", "category": "nonsense" }],
                    "transitions": []
                }
            }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["code"], "workflow.invalid");
    assert!(body["details"]["problems"].is_array(), "{body}");

    let (s, _) = app.delete(&app.admin, &format!("/v1/workflows/{id}")).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = app.get(&app.admin, &format!("/v1/workflows/{id}")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

/// Moving a node is not a workflow change: the layout round-trips and emits no
/// `workflow_changed` event, so a board does not refetch because someone dragged
/// a box.
#[tokio::test]
async fn workflow_layout_round_trips_without_emitting_an_event() {
    let app = TestApp::spawn().await;
    let (_, before) = app.get(&app.admin, "/v1/events?since=0").await;
    let cursor = before["cursor"].as_i64().unwrap_or(0);

    let (s, body) = app
        .put(
            &app.admin,
            "/v1/projects/tp/workflow-layout",
            json!({ "layout": { "todo": { "x": 10, "y": 20 } } }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");

    let (s, got) = app.get(&app.admin, "/v1/projects/tp/workflow-layout").await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(got["layout"]["todo"]["x"], 10, "{got}");

    let (_, after) = app
        .get(&app.admin, &format!("/v1/events?since={cursor}"))
        .await;
    let kinds: Vec<&str> = after["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["kind"].as_str())
        .collect();
    assert!(
        !kinds.contains(&"workflow_changed"),
        "a layout write must not read as a workflow change: {kinds:?}"
    );
}

// `--epic` on all three read surfaces. The subtree walk is the point: an epic's
// work nests, so a one-level `parent` filter would miss most of it.
#[tokio::test]
async fn epic_filter_scopes_roadmap_impact_and_list_to_the_subtree() {
    let app = TestApp::spawn().await;

    // mine:  child -> grandchild        (two levels, so `parent` alone misses one)
    // other: a second epic that must never appear under mine's filter
    let mine = app.create_typed("Mine", "epic", None).await;
    let child = app.create_typed("Child", "task", Some(&mine)).await;
    let grandchild = app.create_typed("Grandchild", "task", Some(&child)).await;
    let other = app.create_typed("Other", "epic", None).await;
    let outsider = app.create_typed("Outsider", "task", Some(&other)).await;

    // One blocker OUTSIDE mine, holding one ticket INSIDE it — the case the
    // filter must not hide, because that is the ticket worth closing.
    let external = app.create_typed("External blocker", "task", None).await;
    for (ticket, blocker) in [(&child, &external), (&outsider, &external)] {
        let (s, b) = app
            .post(
                &app.admin,
                &format!("/v1/tickets/{ticket}/deps"),
                json!({ "blocked_by": blocker }),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "dep add failed: {b}");
    }

    // roadmap?epic= -> that epic alone, and NO project-wide unparented bucket.
    let (status, body) = app
        .get(&app.admin, &format!("/v1/projects/tp/roadmap?epic={mine}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let epics = body["epics"].as_array().expect("epics array");
    assert_eq!(epics.len(), 1, "one epic when filtered: {body}");
    assert_eq!(epics[0]["id"], mine.as_str());
    assert_eq!(epics[0]["total"], 2, "child + grandchild: {body}");
    assert_eq!(body["epic"], mine.as_str(), "filter echoed: {body}");
    assert!(
        body.get("unparented").is_none(),
        "unparented is project-wide and must be omitted under an epic filter: {body}"
    );
    // The initiative side is project-wide for the same reason, and worse: a lane
    // SPANS versions, so reporting lanes beside one version invites reading the
    // lane's numbers as that version's.
    assert!(
        body.get("initiatives").is_none() && body.get("uninitiated").is_none(),
        "the initiative side is project-wide and must be omitted too: {body}"
    );

    // impact?epic= -> counts only work inside the subtree, but still names the
    // OUTSIDE blocker holding it.
    let (status, body) = app
        .get(&app.admin, &format!("/v1/projects/tp/impact?epic={mine}"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["blocked_total"], 2,
        "child + grandchild blocked inside the subtree; outsider excluded: {body}"
    );
    let top = &body["blockers"][0];
    assert_eq!(
        top["id"],
        external.as_str(),
        "external blocker named: {body}"
    );
    assert_eq!(
        top["unblocks"], 2,
        "counts only what it releases INSIDE the epic, not `outsider`: {top}"
    );

    // Unfiltered, the same blocker releases the outsider too — proving the
    // filter narrowed the count rather than the assertion being vacuous.
    let (_, body) = app.get(&app.admin, "/v1/projects/tp/impact").await;
    assert_eq!(
        body["blockers"][0]["unblocks"], 3,
        "unfiltered count includes the outsider: {body}"
    );

    // ls?epic= -> the whole subtree, not just direct children, and the epic
    // itself is the container rather than a member.
    let (status, body) = app
        .get(&app.admin, &format!("/v1/tickets?epic={mine}&limit=50"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let ids: Vec<&str> = body["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|t| t["id"].as_str().expect("id"))
        .collect();
    assert!(
        ids.contains(&child.as_str()),
        "direct child listed: {ids:?}"
    );
    assert!(
        ids.contains(&grandchild.as_str()),
        "GRANDchild listed — a one-level parent filter would miss it: {ids:?}"
    );
    assert!(
        !ids.contains(&mine.as_str()),
        "epic itself excluded: {ids:?}"
    );
    assert!(
        !ids.contains(&outsider.as_str()),
        "another epic's work excluded: {ids:?}"
    );

    // A filter naming a non-epic, or an epic of another project, is a 404 — an
    // empty report cannot tell a typo from a legitimately empty epic.
    let (status, _) = app
        .get(&app.admin, &format!("/v1/projects/tp/roadmap?epic={child}"))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "a task is not an epic");
    let (status, _) = app
        .get(&app.admin, "/v1/projects/tp/impact?epic=tp-nope")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "unknown epic");
}

// ---------------------------------------------------------------------------
// Epic claims: claiming an epic reserves its whole subtree for the holder.

/// The core shape: no `ttl_seconds` on an epic = a claim with no expiry, other
/// actors bounce off every ticket underneath (`claim.epic_held`), the holder
/// works children with ordinary leased claims, and a fenced release lifts the
/// reservation.
#[tokio::test]
async fn epic_claim_without_ttl_never_expires_and_reserves_subtree() {
    let app = TestApp::spawn().await;
    let epic = app.create_typed("Payments revamp", "epic", None).await;
    let c1 = app
        .create_typed("Migrate charges", "task", Some(&epic))
        .await;
    let c2 = app
        .create_typed("Migrate refunds", "task", Some(&epic))
        .await;
    app.to_ready(&epic).await;
    app.to_ready(&c1).await;
    app.to_ready(&c2).await;

    let (s, lease) = app
        .post(&app.worker, &format!("/v1/tickets/{epic}/claim"), json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{lease}");
    assert!(
        lease["expires_at"].is_null(),
        "an epic claimed without a TTL must not expire: {lease}"
    );
    let fence = lease["fence"].as_i64().expect("fence");

    let (_, t) = app.get(&app.worker, &format!("/v1/tickets/{epic}")).await;
    assert_eq!(t["claim"]["holder"], "agent:w1", "{t}");
    assert!(t["claim"]["expires_at"].is_null(), "{t}");
    assert!(t["claim"]["since"].is_string(), "{t}");

    // Another worker cannot claim anywhere in the subtree.
    let (s, err) = app
        .post(&app.worker2, &format!("/v1/tickets/{c1}/claim"), json!({}))
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{err}");
    assert_eq!(err["code"], "claim.epic_held", "{err}");
    assert_eq!(err["details"]["epic"], json!(epic), "{err}");
    assert_eq!(err["details"]["holder"], "agent:w1", "{err}");
    assert!(err["details"]["held_since"].is_string(), "{err}");

    // The holder keeps working children with ordinary leased claims.
    let (s, child_lease) = app
        .post(&app.worker, &format!("/v1/tickets/{c1}/claim"), json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{child_lease}");
    assert!(
        child_lease["expires_at"].is_string(),
        "a child claim stays leased even under an indefinite epic claim: {child_lease}"
    );

    // A fenced release ends the reservation.
    let (s, b) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{epic}/release"),
            json!({ "fence": fence }),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{b}");
    app.claim_as(&app.worker2, &c2).await;
}

/// An explicit TTL bounds an epic claim like any other claim, and the project
/// maximum still applies.
#[tokio::test]
async fn epic_claim_with_explicit_ttl_stays_leased() {
    let app = TestApp::spawn().await;
    let epic = app.create_typed("Bounded lock", "epic", None).await;
    app.to_ready(&epic).await;

    let (s, err) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{epic}/claim"),
            json!({ "ttl_seconds": 4000 }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{err}");
    assert_eq!(err["code"], "validation.ttl", "{err}");

    let (s, lease) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{epic}/claim"),
            json!({ "ttl_seconds": 60 }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{lease}");
    assert!(lease["expires_at"].is_string(), "{lease}");
}

/// A heartbeat on an indefinite claim answers with the lease as it stands and
/// never writes a TTL — a harness that beats on a schedule must not quietly
/// convert "held until released" into a 15-minute lease.
#[tokio::test]
async fn heartbeat_on_an_indefinite_epic_claim_is_a_noop() {
    let app = TestApp::spawn().await;
    let epic = app.create_typed("Long hold", "epic", None).await;
    app.to_ready(&epic).await;
    let (s, lease) = app
        .post(&app.worker, &format!("/v1/tickets/{epic}/claim"), json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{lease}");
    let fence = lease["fence"].as_i64().expect("fence");

    for body in [
        json!({ "fence": fence }),
        json!({ "fence": fence, "ttl_seconds": 60 }),
    ] {
        let (s, beat) = app
            .post(&app.worker, &format!("/v1/tickets/{epic}/heartbeat"), body)
            .await;
        assert_eq!(s, StatusCode::OK, "{beat}");
        assert!(
            beat["expires_at"].is_null(),
            "a beat must not add an expiry: {beat}"
        );
    }
    let (_, t) = app.get(&app.worker, &format!("/v1/tickets/{epic}")).await;
    assert!(t["claim"]["expires_at"].is_null(), "{t}");
}

/// The reservation never displaces live work: while anyone else holds a claim
/// inside the subtree, a fresh epic claim is refused and the ready queue does
/// not offer the epic either.
#[tokio::test]
async fn epic_claim_never_displaces_live_work_inside() {
    let app = TestApp::spawn().await;
    let epic = app.create_typed("Contested epic", "epic", None).await;
    let child = app.create_typed("Live work", "task", Some(&epic)).await;
    app.to_ready(&epic).await;
    app.to_ready(&child).await;
    let child_fence = app.claim_as(&app.worker2, &child).await;

    let (s, err) = app
        .post(&app.worker, &format!("/v1/tickets/{epic}/claim"), json!({}))
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{err}");
    assert_eq!(err["code"], "claim.children_held", "{err}");
    assert_eq!(err["details"]["held"][0]["ticket"], json!(child), "{err}");
    assert_eq!(err["details"]["held"][0]["holder"], "agent:w2", "{err}");

    // The queue agrees with the claim: what the claim-by-id path refuses, the
    // queue must not hand out.
    let (_, peek) = app
        .get(&app.worker, "/v1/ready?project=tp&type=epic&limit=200")
        .await;
    assert_eq!(peek["total"], json!(0), "{peek}");

    let (s, b) = app
        .post(
            &app.worker2,
            &format!("/v1/tickets/{child}/release"),
            json!({ "fence": child_fence }),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{b}");
    let (s, lease) = app
        .post(&app.worker, &format!("/v1/tickets/{epic}/claim"), json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{lease}");
}

/// The shared ready queue stops offering a claimed epic's subtree — and the
/// total stays consistent with the page, so "n of m" never counts tickets the
/// page would refuse to hand out.
#[tokio::test]
async fn ready_queue_stops_offering_a_claimed_epics_subtree() {
    let app = TestApp::spawn().await;
    let epic = app.create_typed("Reserved epic", "epic", None).await;
    let c1 = app
        .create_typed("Reserved child 1", "task", Some(&epic))
        .await;
    let c2 = app
        .create_typed("Reserved child 2", "task", Some(&epic))
        .await;
    let other = app.create_ticket("Unrelated ready work").await;
    for id in [&epic, &c1, &c2, &other] {
        app.to_ready(id).await;
    }

    let (_, before) = app.get(&app.worker, "/v1/ready?project=tp&limit=200").await;
    assert_eq!(before["total"], json!(4), "{before}");

    let (s, lease) = app
        .post(&app.worker, &format!("/v1/tickets/{epic}/claim"), json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{lease}");
    let fence = lease["fence"].as_i64().expect("fence");

    let (_, during) = app.get(&app.worker, "/v1/ready?project=tp&limit=200").await;
    assert_eq!(
        during["total"],
        json!(1),
        "epic + subtree must leave the queue: {during}"
    );
    assert_eq!(during["items"][0]["id"], json!(other), "{during}");

    // ready/claim hands out the unrelated ticket, then nothing — never a
    // reserved child.
    let (s, got) = app
        .post(&app.worker2, "/v1/ready/claim", json!({ "project": "tp" }))
        .await;
    assert_eq!(s, StatusCode::OK, "{got}");
    assert_eq!(got["id"], json!(other), "{got}");
    let (s, _) = app
        .post(&app.worker2, "/v1/ready/claim", json!({ "project": "tp" }))
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // Release: the subtree returns.
    let (s, _) = app
        .post(
            &app.worker,
            &format!("/v1/tickets/{epic}/release"),
            json!({ "fence": fence }),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, after) = app.get(&app.worker, "/v1/ready?project=tp&limit=200").await;
    assert_eq!(after["total"], json!(3), "{after}");
}

/// Admin force-release is the escape hatch for an abandoned indefinite claim:
/// it displaces the holder, bumps the fence, and frees the subtree.
#[tokio::test]
async fn force_release_displaces_an_epic_claim_and_frees_the_subtree() {
    let app = TestApp::spawn().await;
    let epic = app.create_typed("Abandoned epic", "epic", None).await;
    let child = app.create_typed("Waiting work", "task", Some(&epic)).await;
    app.to_ready(&epic).await;
    app.to_ready(&child).await;
    let (s, lease) = app
        .post(&app.worker, &format!("/v1/tickets/{epic}/claim"), json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{lease}");

    let (s, err) = app
        .post(
            &app.worker2,
            &format!("/v1/tickets/{child}/claim"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{err}");

    let (s, forced) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{epic}/force-release"),
            json!({ "reason": "no movement for hours" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{forced}");
    assert_eq!(forced["previous_holder"], "agent:w1", "{forced}");
    assert_eq!(
        forced["lease_expired"],
        json!(false),
        "an indefinite claim is displaced live, never 'already lapsed': {forced}"
    );

    app.claim_as(&app.worker2, &child).await;
}

/// Entering a terminal-category state auto-releases the claim — an epic driven
/// to done can never stay locked.
#[tokio::test]
async fn epic_claim_auto_releases_on_terminal_entry() {
    let app = TestApp::spawn().await;
    let epic = app.create_typed("Finished epic", "epic", None).await;
    app.to_ready(&epic).await;
    let (s, lease) = app
        .post(&app.worker, &format!("/v1/tickets/{epic}/claim"), json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{lease}");
    let fence = lease["fence"].as_i64().expect("fence");

    for to in ["implementing", "review"] {
        let (s, b) = app
            .post(
                &app.worker,
                &format!("/v1/tickets/{epic}/transition"),
                json!({ "to": to, "fence": fence }),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "->{to} failed: {b}");
    }
    let (s, b) = app.transition(&app.human, &epic, "done").await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (_, t) = app.get(&app.worker, &format!("/v1/tickets/{epic}")).await;
    assert!(t["claim"].is_null(), "terminal entry must release: {t}");
}

/// GET /v1/tickets/{id}/claim: who holds it, for how long, and what moved in
/// the subtree since — created / closed are since-the-claim counts, in_progress
/// / blocked are current snapshots, and idle_seconds is the liveness signal an
/// indefinite claim is judged by.
#[tokio::test]
async fn claim_status_reports_movement_since_the_epic_claim() {
    let app = TestApp::spawn().await;
    let epic = app.create_typed("Watched epic", "epic", None).await;
    let done_child = app
        .create_typed("Will be closed", "task", Some(&epic))
        .await;
    let wip_child = app
        .create_typed("Will be in progress", "task", Some(&epic))
        .await;
    let blocked_child = app
        .create_typed("Will be blocked", "task", Some(&epic))
        .await;
    let blocker = app.create_ticket("External blocker, stays open").await;
    let (s, _) = app
        .post(
            &app.admin,
            &format!("/v1/tickets/{blocked_child}/deps"),
            json!({ "blocked_by": blocker }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    app.to_ready(&epic).await;

    // Unclaimed: nobody, and no movement — there is no anchor to count from.
    let (s, st) = app
        .get(&app.worker, &format!("/v1/tickets/{epic}/claim"))
        .await;
    assert_eq!(s, StatusCode::OK, "{st}");
    assert!(st["holder"].is_null(), "{st}");
    assert!(st["movement"].is_null(), "{st}");

    let (s, lease) = app
        .post(&app.worker, &format!("/v1/tickets/{epic}/claim"), json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{lease}");

    // Movement after the claim: one ticket created, one closed, one driven to
    // in-progress; the dep-blocked one just sits there.
    let _created_after = app
        .create_typed("Split out after the claim", "task", Some(&epic))
        .await;
    app.drive_to_done(&done_child).await;
    app.to_implementing(&wip_child).await;

    let (s, st) = app
        .get(&app.worker, &format!("/v1/tickets/{epic}/claim"))
        .await;
    assert_eq!(s, StatusCode::OK, "{st}");
    assert_eq!(st["ticket"], json!(epic), "{st}");
    assert_eq!(st["type"], "epic", "{st}");
    assert_eq!(st["holder"], "agent:w1", "{st}");
    assert_eq!(st["indefinite"], json!(true), "{st}");
    assert!(st["expires_at"].is_null(), "{st}");
    assert!(st["held_since"].is_string(), "{st}");
    assert!(
        st["held_for_seconds"].as_i64().expect("held_for") >= 0,
        "{st}"
    );

    let m = &st["movement"];
    assert_eq!(m["created"], json!(1), "one ticket created since: {st}");
    assert_eq!(m["closed"], json!(1), "one ticket closed since: {st}");
    assert_eq!(
        m["in_progress"],
        json!(1),
        "one currently in progress: {st}"
    );
    assert_eq!(m["blocked"], json!(1), "one currently dep-blocked: {st}");
    assert_eq!(
        m["open"],
        json!(3),
        "wip + blocked + created-after are still open: {st}"
    );
    assert!(m["last_activity_at"].is_string(), "{st}");
    assert!(m["idle_seconds"].as_i64().expect("idle") >= 0, "{st}");
}

// ---------------------------------------------------------------------------
// The people directory, and addressing a decision to a person.
//
// The load-bearing tests here are the authority ones. A named assignee may
// answer an `approve`, which makes the token→person binding an authorization
// fact rather than a display name — so what must be proven is not only that the
// right person can approve, but that nobody else can arrive at that authority by
// another road: not by forging a scope, not by relaying, not by minting somebody
// else's answer link, and not after being disabled.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn user_directory_crud_and_disable_is_a_gate_not_a_delete() {
    let app = TestApp::spawn().await;

    // Adding a person needs admin: this is the directory assignment draws from.
    let (s, denied) = app
        .post(&app.human, "/v1/users", json!({ "handle": "ada" }))
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{denied}");
    assert_eq!(denied["code"], "auth.scope");

    let (s, ada) = app
        .post(
            &app.admin,
            "/v1/users",
            json!({ "handle": "ada", "name": "Ada Lovelace", "email": "ada@example.com", "projects": ["tp"] }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{ada}");
    assert!(ada["id"].as_str().unwrap().starts_with("usr-"), "{ada}");
    assert_eq!(ada["projects"], json!(["tp"]), "{ada}");
    // The handle doubles as a tag reference — that shared rule is what keeps
    // `person:ada` on a ticket pointing at this row.
    assert_eq!(ada["ref"], "person:ada", "{ada}");

    // A second row for the same person is a conflict, not a silent duplicate.
    let (s, dup) = app
        .post(&app.admin, "/v1/users", json!({ "handle": "ada" }))
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{dup}");
    assert_eq!(dup["code"], "user.exists");

    // A handle that could not be a tag reference is refused, with the rule.
    let (s, bad) = app
        .post(&app.admin, "/v1/users", json!({ "handle": "Ada Lovelace" }))
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    assert_eq!(bad["code"], "validation.user_handle");

    // Reads are open to any valid token: a name has to be renderable by whoever
    // is looking at a question.
    let (s, one) = app.get(&app.worker, "/v1/users/ada").await;
    assert_eq!(s, StatusCode::OK, "{one}");
    assert_eq!(one["name"], "Ada Lovelace");

    // Rename, and clear the email with an explicit null.
    let (s, patched) = app
        .patch(
            &app.admin,
            "/v1/users/ada",
            json!({ "name": "Ada L.", "email": null }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{patched}");
    assert_eq!(patched["name"], "Ada L.");
    assert!(patched["email"].is_null(), "{patched}");

    // Disabling hides her from the default listing but keeps every record
    // resolvable — there is no delete, because `answered_by: ada` must not
    // become unreadable after the fact.
    let (s, disabled) = app
        .post(&app.admin, "/v1/users/ada/disable", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{disabled}");
    assert_eq!(disabled["disabled"], json!(true), "{disabled}");

    let (_, list) = app.get(&app.admin, "/v1/users").await;
    assert_eq!(list["items"].as_array().unwrap().len(), 0, "{list}");
    let (_, all) = app.get(&app.admin, "/v1/users?include_disabled=1").await;
    assert_eq!(all["items"].as_array().unwrap().len(), 1, "{all}");
    let (s, still_there) = app.get(&app.worker, "/v1/users/ada").await;
    assert_eq!(s, StatusCode::OK, "a disabled person still resolves");
    assert_eq!(still_there["handle"], "ada", "{still_there}");

    let (s, back) = app
        .post(&app.admin, "/v1/users/ada/enable", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{back}");
    assert_eq!(back["disabled"], json!(false), "{back}");
}

#[tokio::test]
async fn user_is_never_created_implicitly_unlike_a_tag() {
    // Tagging a ticket `person:nobody` registers a stub tag on the fly. The
    // directory deliberately does not do that: an assignable person conjured from
    // a typo looks exactly like a real one, and work would sit waiting on them.
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Typo target").await;
    let fence = app.to_implementing(&id).await;

    let (s, missing) = app
        .post(
            &app.worker,
            "/v1/questions",
            json!({ "ticket": id, "kind": "clarify", "title": "Who?", "assignee": "nobdy", "fence": fence }),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{missing}");
    assert_eq!(missing["code"], "notfound.user");
    assert!(
        missing["remedy"]
            .as_str()
            .unwrap()
            .contains("POST /v1/users"),
        "the refusal must name how to add the person: {missing}"
    );
}

#[tokio::test]
async fn users_list_is_a_bounded_envelope() {
    let app = TestApp::spawn().await;
    for i in 0..5 {
        let (s, b) = app
            .post(
                &app.admin,
                "/v1/users",
                json!({ "handle": format!("p{i}"), "projects": ["tp"] }),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{b}");
    }
    let (s, page) = app.get(&app.admin, "/v1/users?limit=2").await;
    assert_eq!(s, StatusCode::OK, "{page}");
    assert_eq!(page["items"].as_array().unwrap().len(), 2, "{page}");
    assert_eq!(
        page["total"],
        json!(5),
        "the count is the whole match: {page}"
    );
    assert_eq!(page["limit"], json!(2), "{page}");
    assert!(
        page["note"].as_str().unwrap().contains("Showing 2 of 5"),
        "a short page must say so in prose: {page}"
    );
    // Ordering is by handle, so an offset is a stable continuation.
    let (_, second) = app.get(&app.admin, "/v1/users?limit=2&offset=2").await;
    assert_eq!(second["items"][0]["handle"], "p2", "{second}");
    // A whole page carries no note to mistake for one.
    let (_, whole) = app.get(&app.admin, "/v1/users").await;
    assert!(whole["note"].is_null(), "{whole}");
}

#[tokio::test]
async fn assignment_requires_an_active_member_of_the_project() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Address me").await;
    let fence = app.to_implementing(&id).await;

    // In the directory, but not on this project: membership is what says who may
    // be handed work here.
    app.add_user("outsider", None);
    let (s, refused) = app
        .post(
            &app.worker,
            "/v1/questions",
            json!({ "ticket": id, "kind": "clarify", "title": "Which key?", "assignee": "outsider", "fence": fence }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["code"], "user.not_member");

    // A member works, and the question comes back with the person resolved
    // rather than as an opaque id.
    app.add_user("ada", Some("tp"));
    let (qid, body) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "clarify", "title": "Which key?", "assignee": "ada", "fence": fence }),
        )
        .await;
    assert_eq!(body["question"]["assignee"]["handle"], "ada", "{body}");
    assert_eq!(
        body["question"]["assignee"]["label"], "ada the tester",
        "the inbox needs a name, not an id: {body}"
    );

    // A disabled person cannot be newly addressed.
    app.post(&app.admin, "/v1/users/ada/disable", json!({}))
        .await;
    let (s, gone) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/assign"),
            json!({ "assignee": "ada" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{gone}");
    assert_eq!(gone["code"], "user.disabled");
}

#[tokio::test]
async fn assigning_is_routing_not_a_lock() {
    // The reason assignment is deliberately not exclusive: a decision must never
    // be stranded because the person it was addressed to is away. Any `human`
    // token can still answer the three ordinary kinds.
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Routing only").await;
    let fence = app.to_implementing(&id).await;
    app.add_user("ada", Some("tp"));
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "Drop it?", "assignee": "ada", "fence": fence }),
        )
        .await;

    let (s, ok) = app.answer(&app.human, &qid, json!(true)).await;
    assert_eq!(s, StatusCode::OK, "a colleague may still answer: {ok}");
    assert_eq!(ok["ticket"]["state"], "ready", "{ok}");

    // And the decision is on the ticket thread, where a resuming agent reads it.
    let (_, detail) = app
        .get(&app.worker, &format!("/v1/tickets/{id}?include=comments"))
        .await;
    let thread = detail["comments"].as_array().unwrap();
    assert!(
        thread
            .iter()
            .any(|c| c["body"].as_str().unwrap().contains("Human answered")),
        "{detail}"
    );
}

#[tokio::test]
async fn assign_after_the_ask_is_the_usual_case_and_only_while_open() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Triage later").await;
    let fence = app.to_implementing(&id).await;
    app.add_user("ada", Some("tp"));
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "clarify", "title": "Which key?", "fence": fence }),
        )
        .await;

    // An absent field is refused rather than guessed at: unassign and "you forgot
    // the value" must not look the same.
    let (s, bad) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/assign"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    assert_eq!(bad["code"], "validation.assignee");

    let (s, assigned) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/assign"),
            json!({ "assignee": "ada" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{assigned}");
    assert_eq!(assigned["question"]["assignee"]["handle"], "ada");

    // The handover is mirrored onto the ticket and recorded as an event.
    let (_, events) = app
        .get(&app.admin, &format!("/v1/events?since=0&ticket={id}"))
        .await;
    let assign_event = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["kind"] == "question_assigned")
        .cloned()
        .expect("a question_assigned event");
    assert_eq!(assign_event["payload"]["assignee"], "ada", "{assign_event}");
    let (_, thread) = app
        .get(&app.worker, &format!("/v1/tickets/{id}?include=comments"))
        .await;
    assert!(
        thread["comments"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["body"].as_str().unwrap().contains("addressed")),
        "the handover belongs in the thread a resuming agent reads: {thread}"
    );

    // Back to the open pool.
    let (s, cleared) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/assign"),
            json!({ "assignee": null }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{cleared}");
    assert!(cleared["question"]["assignee"].is_null(), "{cleared}");

    // Once decided, who it was waiting on is part of the record.
    app.answer(&app.human, &qid, json!("the original")).await;
    let (s, settled) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/assign"),
            json!({ "assignee": "ada" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{settled}");
    assert_eq!(settled["code"], "question.not_open");
}

#[tokio::test]
async fn approve_must_name_expertise_or_an_assignee() {
    // An approval has to name something that gates it. With neither, it is a
    // `confirm` wearing a stronger word, and the audit trail would record
    // authority nobody held.
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Ungated approve").await;
    let fence = app.to_implementing(&id).await;

    let (s, bad) = app
        .post(
            &app.worker,
            "/v1/questions",
            json!({ "ticket": id, "kind": "approve", "title": "Sign off?", "fence": fence }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    assert_eq!(bad["code"], "validation.expertise");
    assert!(
        bad["message"].as_str().unwrap().contains("assignee"),
        "the refusal must name both gates: {bad}"
    );

    // Naming only a person is enough — that is the new second gate.
    app.add_user("ada", Some("tp"));
    let (_, ok) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "approve", "title": "Sign off?", "assignee": "ada", "fence": fence }),
        )
        .await;
    assert_eq!(ok["question"]["assignee"]["handle"], "ada", "{ok}");
}

#[tokio::test]
async fn the_named_assignee_may_approve_and_a_plain_human_may_not() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Named approval").await;
    let fence = app.to_implementing(&id).await;
    let ada = app.add_user("ada", Some("tp"));
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "approve", "title": "Ship it?", "assignee": "ada", "fence": fence }),
        )
        .await;

    // A colleague with `human` — even an admin — is not the person who was asked.
    for token in [&app.human, &app.admin] {
        let (s, denied) = app.answer(token, &qid, json!(true)).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{denied}");
        assert_eq!(denied["code"], "question.approve_expertise");
        assert!(
            denied["message"].as_str().unwrap().contains("addressed to"),
            "the refusal must say which door to knock on: {denied}"
        );
    }

    // A credential bound to Ada is Ada, and that is the whole authority here —
    // note it carries no expert scope at all.
    let ada_token = app.mint_as_user("human:ada", &["read", "write", "human"], &ada);
    let (s, ok) = app.answer(&ada_token, &qid, json!(true)).await;
    assert_eq!(s, StatusCode::OK, "{ok}");
    assert_eq!(ok["ticket"]["state"], "ready", "{ok}");
    assert_eq!(ok["question"]["answered_by"], "human:ada", "{ok}");
}

#[tokio::test]
async fn a_user_scope_string_cannot_forge_assignee_identity() {
    // The trap this design has to survive. Scopes are free-form — `expert:<tag>`
    // proves it — so an admin can mint a token carrying literally `user:usr-…`.
    // If identity were read from the scope set, that string would BE Ada, and
    // minting it would hand out her approval authority. Identity is carried
    // separately (`tokens.user`, admin-set) for exactly this reason.
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Forged identity").await;
    let fence = app.to_implementing(&id).await;
    let ada = app.add_user("ada", Some("tp"));
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "approve", "title": "Ship it?", "assignee": "ada", "fence": fence }),
        )
        .await;

    let forged = app.mint(
        "human:impostor",
        &[
            "read",
            "write",
            "human",
            // Both spellings someone might reach for.
            &format!("user:{ada}"),
            "user:ada",
        ],
        None,
    );
    let (s, denied) = app.answer(&forged, &qid, json!(true)).await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "a scope string must never be an identity: {denied}"
    );
    assert_eq!(denied["code"], "question.approve_expertise");

    // And the question is still open for the person who was actually asked.
    let (_, q) = app.get(&app.admin, &format!("/v1/questions/{qid}")).await;
    assert_eq!(q["status"], "open", "{q}");
}

#[tokio::test]
async fn a_disabled_assignee_loses_approve_authority_but_past_answers_resolve() {
    let app = TestApp::spawn().await;
    let ada = app.add_user("ada", Some("tp"));
    let ada_token = app.mint_as_user("human:ada", &["read", "write", "human"], &ada);

    // First, an approval she gives while active — the record we must not lose.
    let first = app.create_ticket("Before").await;
    let fence = app.to_implementing(&first).await;
    let (q1, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": first, "kind": "approve", "title": "Ship one?", "assignee": "ada", "fence": fence }),
        )
        .await;
    let (s, ok) = app.answer(&ada_token, &q1, json!(true)).await;
    assert_eq!(s, StatusCode::OK, "{ok}");

    // A second, still open, when she is disabled.
    let second = app.create_ticket("After").await;
    let fence2 = app.to_implementing(&second).await;
    let (q2, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": second, "kind": "approve", "title": "Ship two?", "assignee": "ada", "fence": fence2 }),
        )
        .await;
    app.post(&app.admin, "/v1/users/ada/disable", json!({}))
        .await;

    // Her token still authenticates — disabling is not revocation — but the
    // authority that came from being the named person is gone.
    let (s, denied) = app.answer(&ada_token, &q2, json!(true)).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{denied}");
    assert_eq!(denied["code"], "question.approve_expertise");

    // The earlier decision still reads back, with her name on it: this is why
    // there is no delete.
    let (s, settled) = app.get(&app.worker, &format!("/v1/questions/{q1}")).await;
    assert_eq!(s, StatusCode::OK, "{settled}");
    assert_eq!(settled["answered_by"], "human:ada", "{settled}");
    assert_eq!(settled["assignee"]["handle"], "ada", "{settled}");
    assert_eq!(settled["assignee"]["disabled"], json!(true), "{settled}");
}

#[tokio::test]
async fn relaying_an_approve_stays_refused_when_a_person_gates_it() {
    // A relay records a decision made out of band, so the name in it is a claim
    // the relayer is making rather than one the server can vouch for. That is
    // exactly what an `approve` may not accept — including when the gate is a
    // person rather than a domain.
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Relayed approval").await;
    let fence = app.to_implementing(&id).await;
    let ada = app.add_user("ada", Some("tp"));
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "approve", "title": "Ship it?", "assignee": "ada", "fence": fence }),
        )
        .await;

    // Even a relayer whose own credential is bound to Ada cannot relay it: the
    // channel is the problem, not the identity.
    let relayer = app.mint_as_user("orch:main", &["read", "write", "answer:relay"], &ada);
    let (s, denied) = app
        .post(
            &relayer,
            &format!("/v1/questions/{qid}/answer"),
            json!({ "answer": true, "on_behalf_of": "human:ada" }),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{denied}");
    assert_eq!(denied["code"], "answer.relay_approve");
}

#[tokio::test]
async fn an_answer_link_for_a_person_gated_approve_is_only_mintable_by_them() {
    // You can only delegate authority you hold, and whoever holds the link string
    // can spend it — so a colleague minting "Ada's" link would be approving as
    // Ada. Admin is not a way round it either: in this codebase admin has never
    // stood in for the authority a gate names.
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Delegated approval").await;
    let fence = app.to_implementing(&id).await;
    let ada = app.add_user("ada", Some("tp"));
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "approve", "title": "Ship it?", "assignee": "ada", "fence": fence }),
        )
        .await;

    for token in [&app.human, &app.admin] {
        let (s, denied) = app
            .post(
                token,
                &format!("/v1/questions/{qid}/answer-link"),
                json!({}),
            )
            .await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{denied}");
        assert_eq!(denied["code"], "question.approve_expertise");
    }

    // Ada mints her own link — to answer from a phone — and it carries her
    // identity, which is what satisfies the gate when there is no expertise tag
    // to synthesize a scope from.
    let ada_token = app.mint_as_user("human:ada", &["read", "write", "human"], &ada);
    let (s, link) = app
        .post(
            &ada_token,
            &format!("/v1/questions/{qid}/answer-link"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{link}");
    assert_eq!(link["user"], ada, "the grant carries her identity: {link}");
    assert_eq!(link["actor"], "ada", "attributed to her, not to the link");

    let grant = link["token"].as_str().unwrap().to_string();
    let (s, answered) = app
        .post(&grant, "/v1/answer/self", json!({ "answer": true }))
        .await;
    assert_eq!(s, StatusCode::OK, "{answered}");
    assert_eq!(answered["ticket"]["state"], "ready", "{answered}");
}

#[tokio::test]
async fn mine_unions_the_person_asked_and_the_expertise_covered() {
    // "Waiting on me" has two senses on this board and a person owes an answer on
    // either, so they are ORed. Requiring both would hide a question aimed
    // straight at someone because it happened to carry no tag.
    let app = TestApp::spawn().await;
    let ada = app.add_user("ada", Some("tp"));

    let addressed = app.create_ticket("Addressed to her").await;
    let f1 = app.to_implementing(&addressed).await;
    app.ask(
        &app.worker,
        json!({ "ticket": addressed, "kind": "clarify", "title": "Addressed", "assignee": "ada", "fence": f1 }),
    )
    .await;

    let by_domain = app.create_ticket("In her domain").await;
    let f2 = app.to_implementing(&by_domain).await;
    app.ask(
        &app.worker,
        json!({ "ticket": by_domain, "kind": "clarify", "title": "Domain", "expertise": ["domain:billing"], "fence": f2 }),
    )
    .await;

    let neither = app.create_ticket("Nobody's").await;
    let f3 = app.to_implementing(&neither).await;
    app.ask(
        &app.worker,
        json!({ "ticket": neither, "kind": "clarify", "title": "Unowned", "fence": f3 }),
    )
    .await;

    let ada_token = app.mint_as_user(
        "human:ada",
        &["read", "write", "human", "expert:domain:billing"],
        &ada,
    );
    let (s, mine) = app.get(&ada_token, "/v1/questions?mine=true").await;
    assert_eq!(s, StatusCode::OK, "{mine}");
    let titles: Vec<&str> = mine["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| q["title"].as_str().unwrap())
        .collect();
    assert!(titles.contains(&"Addressed"), "{mine}");
    assert!(titles.contains(&"Domain"), "{mine}");
    assert!(!titles.contains(&"Unowned"), "{mine}");

    // One person's queue, by name — including somebody else's, which is the
    // point of a shared board.
    let (_, hers) = app.get(&app.human, "/v1/questions?assignee=ada").await;
    let hers_titles: Vec<&str> = hers["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| q["title"].as_str().unwrap())
        .collect();
    assert_eq!(hers_titles, vec!["Addressed"], "{hers}");

    // And the triage read: what nobody has been asked yet.
    let (_, open_pool) = app.get(&app.human, "/v1/questions?assignee=none").await;
    let pool_titles: Vec<&str> = open_pool["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| q["title"].as_str().unwrap())
        .collect();
    assert!(pool_titles.contains(&"Domain"), "{open_pool}");
    assert!(pool_titles.contains(&"Unowned"), "{open_pool}");
    assert!(!pool_titles.contains(&"Addressed"), "{open_pool}");

    // A token that is nobody and covers nothing is told so, rather than shown an
    // empty queue it might read as "all clear".
    let (_, nothing) = app.get(&app.worker, "/v1/questions?mine=true").await;
    assert!(
        nothing["note"]
            .as_str()
            .unwrap()
            .contains("not bound to a person"),
        "{nothing}"
    );
}

#[tokio::test]
async fn whoami_reports_the_person_behind_the_credential() {
    let app = TestApp::spawn().await;
    let ada = app.add_user("ada", Some("tp"));
    let bound = app.mint_as_user("human:ada", &["read", "human"], &ada);

    let (s, me) = app.get(&bound, "/v1/whoami").await;
    assert_eq!(s, StatusCode::OK, "{me}");
    assert_eq!(me["user"]["handle"], "ada", "{me}");
    assert_eq!(me["actor"], "human:ada", "actor stays what it was: {me}");

    // A machine token is nobody, and says so rather than omitting the field.
    let (_, machine) = app.get(&app.worker, "/v1/whoami").await;
    assert!(machine["user"].is_null(), "{machine}");

    // Binding is admin-only — it is an authorization act — and refuses an unknown
    // or disabled person rather than storing a dangling string.
    let (s, bad) = app
        .post(
            &app.admin,
            "/v1/tokens",
            json!({ "actor": "human:ghost", "scopes": ["read"], "user": "nobody" }),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{bad}");
    assert_eq!(bad["code"], "notfound.user");

    app.post(&app.admin, "/v1/users/ada/disable", json!({}))
        .await;
    let (s, refused) = app
        .post(
            &app.admin,
            "/v1/tokens",
            json!({ "actor": "human:ada2", "scopes": ["read"], "user": "ada" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["code"], "user.disabled");
}

#[tokio::test]
async fn archiving_a_project_freezes_memberships_but_not_the_directory() {
    // Users are global, so the archive gate applies to the membership edge rather
    // than to the person: the directory stays readable and editable, while who may
    // be handed work in a frozen project cannot change.
    let app = TestApp::spawn().await;
    app.add_user("ada", Some("tp"));
    app.post(&app.admin, "/v1/projects/tp/archive", json!({}))
        .await;

    let (s, frozen) = app
        .post(
            &app.admin,
            "/v1/users/ada/projects",
            json!({ "project": "tp" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{frozen}");
    assert_eq!(frozen["code"], "project.archived");

    let (s, removal) = app.delete(&app.admin, "/v1/users/ada/projects/tp").await;
    assert_eq!(s, StatusCode::CONFLICT, "{removal}");
    assert_eq!(removal["code"], "project.archived");

    // The person is untouched: readable, renameable, and still resolvable from
    // every record that names them.
    let (s, still) = app.get(&app.worker, "/v1/users/ada").await;
    assert_eq!(s, StatusCode::OK, "{still}");
    let (s, renamed) = app
        .patch(&app.admin, "/v1/users/ada", json!({ "name": "Ada L." }))
        .await;
    assert_eq!(s, StatusCode::OK, "{renamed}");
}

#[tokio::test]
async fn membership_removal_leaves_an_open_question_addressed_to_them() {
    // Retracting an open decision silently would leave it with nobody looking at
    // it. They simply cannot be handed anything new.
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Still theirs").await;
    let fence = app.to_implementing(&id).await;
    app.add_user("ada", Some("tp"));
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "clarify", "title": "Which key?", "assignee": "ada", "fence": fence }),
        )
        .await;

    let (s, removed) = app.delete(&app.admin, "/v1/users/ada/projects/tp").await;
    assert_eq!(s, StatusCode::OK, "{removed}");

    let (_, q) = app.get(&app.worker, &format!("/v1/questions/{qid}")).await;
    assert_eq!(q["assignee"]["handle"], "ada", "still waiting on her: {q}");

    // But nothing new can be addressed to her here.
    let (s, refused) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/assign"),
            json!({ "assignee": "ada" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["code"], "user.not_member");

    // A second membership removal is a 404, not a silent success.
    let (s, again) = app.delete(&app.admin, "/v1/users/ada/projects/tp").await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{again}");
    assert_eq!(again["code"], "notfound.membership");
}

/// A `person:` tag and the user of the same handle are one person, and the
/// directory is the authority on their name.
///
/// The two registries would otherwise drift in the way that matters to a reader:
/// tagging a ticket lazy-creates a stub whose label is the handle again, so the
/// board would say `ada` while the directory knew "Ada Lovelace" — and nothing
/// would connect them.
#[tokio::test]
async fn a_person_tag_resolves_to_the_directory_person() {
    let app = TestApp::spawn().await;
    app.add_user("ada", Some("tp"));

    // Tagging a ticket registers the stub, exactly as before.
    let id = app.create_ticket("Tagged with a person").await;
    let (s, tagged) = app
        .patch(
            &app.worker,
            &format!("/v1/tickets/{id}"),
            json!({ "tags": ["person:ada", "component:billing", "person:nobody"] }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{tagged}");

    let (s, registry) = app.get(&app.worker, "/v1/projects/tp/tags").await;
    assert_eq!(s, StatusCode::OK, "{registry}");
    let by_ref: std::collections::HashMap<&str, &Value> = registry["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (t["ref"].as_str().unwrap(), t))
        .collect();

    // The person we know: named, and pointing at the same handle.
    let ada = by_ref["person:ada"];
    assert_eq!(ada["person"]["handle"], "ada", "{ada}");
    assert_eq!(ada["person"]["label"], "ada the tester", "{ada}");
    assert_eq!(ada["person"]["disabled"], json!(false), "{ada}");
    // The stub label is untouched — the directory is what resolves, not a copy.
    assert_eq!(ada["label"], "ada", "{ada}");

    // A person handle nobody was added under stays a bare reference rather than
    // inventing somebody.
    assert!(by_ref["person:nobody"]["person"].is_null(), "{registry}");
    // And a tag of another kind is never a person, whatever its handle.
    assert!(
        by_ref["component:billing"]["person"].is_null(),
        "{registry}"
    );

    // Disabling shows up here too: the tag still resolves, and says they can no
    // longer be handed work.
    app.post(&app.admin, "/v1/users/ada/disable", json!({}))
        .await;
    let (_, one) = app
        .get(&app.worker, "/v1/projects/tp/tags/person/ada")
        .await;
    assert_eq!(one["person"]["disabled"], json!(true), "{one}");

    // Searching the registry finds them by the name the directory holds, which the
    // tag row does not carry.
    let (_, found) = app
        .get(
            &app.worker,
            "/v1/projects/tp/tags?q=tester&include_disabled=1",
        )
        .await;
    let refs: Vec<&str> = found["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["ref"].as_str().unwrap())
        .collect();
    assert_eq!(refs, vec!["person:ada"], "{found}");
}

// ---------------------------------------------------------------------------
// Auth/query hardening (takomo-66k9 … takomo-dpue)

#[tokio::test]
async fn root_redirects_to_board_without_auth() {
    let app = TestApp::spawn().await;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("client");
    let resp = client.get(app.url("/")).send().await.expect("GET /");
    assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        resp.headers().get("location").and_then(|v| v.to_str().ok()),
        Some("/board")
    );
}

#[tokio::test]
async fn answer_self_rejects_grant_project_mismatch() {
    let app = TestApp::spawn().await;
    let id = app.create_ticket("Mismatch guard").await;
    let fence = app.to_implementing(&id).await;
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "OK?", "fence": fence }),
        )
        .await;
    let (s, link) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/answer-link"),
            json!({ "actor": "human:outsider" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{link}");
    let token = link["token"].as_str().unwrap().to_string();
    let grant_id = link["id"].as_str().unwrap().to_string();

    let conn = rusqlite::Connection::open(app.tmp.path().join("test.db")).unwrap();
    conn.execute(
        "UPDATE answer_grants SET project = 'wrong' WHERE id = ?1",
        [&grant_id],
    )
    .unwrap();

    let (s, body) = app.get(&token, "/v1/answer/self").await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "answer.project");
    let (s, body) = app
        .post(&token, "/v1/answer/self", json!({ "answer": "yes" }))
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "answer.project");
}

#[tokio::test]
async fn share_and_answer_grant_touch_last_used_at() {
    let app = TestApp::spawn().await;
    let _ = app.create_ticket("Audit touch").await;
    let (_, share) = app
        .post(
            &app.admin,
            "/v1/shares",
            json!({ "kind": "project", "ref": "tp" }),
        )
        .await;
    let share_token = share["token"].as_str().unwrap().to_string();
    let share_id = share["id"].as_str().unwrap().to_string();

    let id = app.create_ticket("Grant touch").await;
    let fence = app.to_implementing(&id).await;
    let (qid, _) = app
        .ask(
            &app.worker,
            json!({ "ticket": id, "kind": "confirm", "title": "Touch?", "fence": fence }),
        )
        .await;
    let (_, link) = app
        .post(
            &app.human,
            &format!("/v1/questions/{qid}/answer-link"),
            json!({ "actor": "human:outsider" }),
        )
        .await;
    let grant_token = link["token"].as_str().unwrap().to_string();
    let grant_id = link["id"].as_str().unwrap().to_string();

    let (s, _) = app.get(&share_token, "/v1/shares/self").await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = app.get(&grant_token, "/v1/answer/self").await;
    assert_eq!(s, StatusCode::OK);

    let store = app.open_store();
    let share_row = store.get_share(&share_id).unwrap().unwrap();
    assert!(
        share_row.last_used_at.is_some(),
        "share last_used_at should be set: {share_row:?}"
    );
    let grant_row = store.get_answer_grant(&grant_id).unwrap().unwrap();
    assert!(
        grant_row.last_used_at.is_some(),
        "grant last_used_at should be set: {grant_row:?}"
    );
}

#[tokio::test]
async fn ticket_search_treats_percent_as_literal() {
    let app = TestApp::spawn().await;
    let (s, t) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "tp", "title": "100% done", "body": "nothing else" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let (s, t2) = app
        .post(
            &app.admin,
            "/v1/tickets",
            json!({ "project": "tp", "title": "Unrelated", "body": "100 done" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{t2}");

    let (_, list) = app.get(&app.admin, "/v1/tickets?project=tp&q=100%25").await;
    let ids: Vec<&str> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 1, "literal % must not act as a wildcard: {list}");
    assert!(list["items"][0]["title"].as_str().unwrap().contains('%'));
}

#[tokio::test]
async fn tag_kind_query_param_is_validated() {
    let app = TestApp::spawn().await;
    let (s, e) = app
        .get(&app.admin, "/v1/tickets?project=tp&tag_kind=per%son")
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert_eq!(e["code"], "validation.tag_kind");
}

// ---------------------------------------------------------------------------
// Mindmaps: thinking out loud, fast, before any of it is an idea.
//
// The tests that matter here are the ones about *shape*: a tree that stays a
// tree (no cycles, bounded depth), a batch that lands whole or not at all, and a
// branch that graduates without leaving the map — because the map is the record
// of how the thinking got there.
// ---------------------------------------------------------------------------

/// The path the feature exists for: a root, four branches, leaves under one of
/// them — in as few calls as somebody talking would make.
#[tokio::test]
async fn a_mindmap_grows_a_branch_at_a_time() {
    let app = TestApp::spawn().await;
    let (s, made) = app
        .post(
            &app.worker,
            "/v1/mindmaps",
            json!({ "project": "tp", "title": "Payments rebuild" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let map = made["mindmap"]["id"].as_str().unwrap().to_string();
    assert_eq!(made["mindmap"]["status"], "open", "{made}");
    assert!(
        made["note"].as_str().unwrap().contains("nodes"),
        "the reply must say how to grow it: {made}"
    );

    // One call, four branches — the shape an agent brainstorming with somebody
    // actually sends.
    let (s, grown) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "nodes": [
                { "text": "API" },
                { "text": "integrations" },
                { "text": "workflows" },
                { "text": "ideas" },
            ] }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{grown}");
    let branches = grown["nodes"].as_array().unwrap();
    assert_eq!(branches.len(), 4, "{grown}");
    // Positions are gapped so a later insert between two is one write.
    assert!(
        branches[0]["position"].as_i64().unwrap() < branches[1]["position"].as_i64().unwrap(),
        "siblings keep the order they were sent in: {grown}"
    );
    let api = branches[0]["id"].as_str().unwrap().to_string();

    // A single thought, unwrapped — what a keystroke sends.
    let (s, one) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "parent": api, "text": "versioning?" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "a bare node needs no array: {one}");

    // One read gives the canvas the whole tree.
    let (s, whole) = app.get(&app.worker, &format!("/v1/mindmaps/{map}")).await;
    assert_eq!(s, StatusCode::OK, "{whole}");
    assert_eq!(whole["total"], json!(5), "{whole}");
    assert_eq!(whole["mindmap"]["nodes"], json!(5), "{whole}");

    // And the outline is the shape a model reads cheapest.
    let (s, outline) = app
        .get(&app.worker, &format!("/v1/mindmaps/{map}/outline"))
        .await;
    assert_eq!(s, StatusCode::OK, "{outline}");
    let text = outline["outline"].as_str().unwrap();
    assert!(text.contains("# Payments rebuild"), "{text}");
    assert!(
        text.contains("- API\n  - versioning?"),
        "nesting shows: {text}"
    );
}

#[tokio::test]
async fn a_node_is_capped_at_a_sentence_or_two_and_the_refusal_says_why() {
    // The cap is the method, not a limitation — so the refusal has to name the way
    // out rather than just report a length.
    let app = TestApp::spawn().await;
    let (_, made) = app
        .post(
            &app.worker,
            "/v1/mindmaps",
            json!({ "project": "tp", "title": "Payments rebuild" }),
        )
        .await;
    let map = made["mindmap"]["id"].as_str().unwrap().to_string();

    let (s, refused) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "text": "x".repeat(281) }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
    assert_eq!(refused["code"], "validation.mindmap_node_text");
    assert!(
        refused["remedy"].as_str().unwrap().contains("promote"),
        "the refusal must point at the way out: {refused}"
    );

    // An empty thought is not one.
    let (s, empty) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "text": "   " }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{empty}");
}

#[tokio::test]
async fn a_batch_lands_whole_or_not_at_all() {
    // Ten nodes from one agent turn are one act of brainstorming. Half of them
    // landing would leave a map nobody asked for and no way to tell which half.
    let app = TestApp::spawn().await;
    let (_, made) = app
        .post(
            &app.worker,
            "/v1/mindmaps",
            json!({ "project": "tp", "title": "Payments rebuild" }),
        )
        .await;
    let map = made["mindmap"]["id"].as_str().unwrap().to_string();

    let (s, refused) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "nodes": [
                { "text": "fine" },
                { "text": "also fine" },
                { "text": "x".repeat(400) },
            ] }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");

    let (_, whole) = app.get(&app.worker, &format!("/v1/mindmaps/{map}")).await;
    assert_eq!(
        whole["total"],
        json!(0),
        "the two valid nodes must not have landed: {whole}"
    );
}

#[tokio::test]
async fn the_tree_stays_a_tree() {
    let app = TestApp::spawn().await;
    let (_, made) = app
        .post(
            &app.worker,
            "/v1/mindmaps",
            json!({ "project": "tp", "title": "Payments rebuild" }),
        )
        .await;
    let map = made["mindmap"]["id"].as_str().unwrap().to_string();
    let (_, grown) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "nodes": [{ "text": "API" }] }),
        )
        .await;
    let api = grown["nodes"][0]["id"].as_str().unwrap().to_string();
    let (_, child) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "parent": api, "text": "versioning?" }),
        )
        .await;
    let versioning = child["nodes"][0]["id"].as_str().unwrap().to_string();

    // The drag that would cut a branch off the map: dropping a node onto its own
    // child. Easy to try with a mouse, impossible to notice afterwards.
    let (s, cycle) = app
        .patch(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{api}"),
            json!({ "parent": versioning }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{cycle}");
    assert_eq!(cycle["code"], "mindmap.cycle");

    // And onto itself.
    let (s, self_cycle) = app
        .patch(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{api}"),
            json!({ "parent": api }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{self_cycle}");

    // A legal move: the child up to the first ring. `parent: null` is an
    // instruction, where an absent field would be "leave it alone".
    let (s, lifted) = app
        .patch(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{versioning}"),
            json!({ "parent": null }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{lifted}");
    assert!(lifted["node"]["parent"].is_null(), "{lifted}");

    // Depth is bounded: eight levels is where a shape stops being readable.
    let mut parent = api.clone();
    for depth in 2..=9 {
        let (s, deeper) = app
            .post(
                &app.worker,
                &format!("/v1/mindmaps/{map}/nodes"),
                json!({ "parent": parent, "text": format!("level {depth}") }),
            )
            .await;
        if depth <= 8 {
            assert_eq!(s, StatusCode::CREATED, "level {depth} should fit: {deeper}");
            parent = deeper["nodes"][0]["id"].as_str().unwrap().to_string();
        } else {
            assert_eq!(
                s,
                StatusCode::CONFLICT,
                "level {depth} is past the cap: {deeper}"
            );
            assert_eq!(deeper["code"], "mindmap.too_deep");
        }
    }
}

#[tokio::test]
async fn placement_is_kept_and_can_be_handed_back_to_the_layout() {
    // A map nobody has dragged stays tidy as it grows; one that has been arranged
    // stays exactly where it was left. Both need to be expressible, and `at: null`
    // is what "tidy up" sends.
    let app = TestApp::spawn().await;
    let (_, made) = app
        .post(
            &app.worker,
            "/v1/mindmaps",
            json!({ "project": "tp", "title": "Payments rebuild" }),
        )
        .await;
    let map = made["mindmap"]["id"].as_str().unwrap().to_string();
    let (_, grown) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "nodes": [{ "text": "API" }] }),
        )
        .await;
    let node = grown["nodes"][0]["id"].as_str().unwrap().to_string();
    assert!(
        grown["nodes"][0]["at"].is_null(),
        "unplaced by default: {grown}"
    );

    let (s, placed) = app
        .patch(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{node}"),
            json!({ "at": { "x": 120.5, "y": -40.0 } }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{placed}");
    assert_eq!(placed["node"]["at"]["x"], json!(120.5), "{placed}");

    // Half a coordinate places nothing, so it is refused rather than guessed at.
    let (s, half) = app
        .patch(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{node}"),
            json!({ "at": { "x": 10.0 } }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{half}");

    let (s, tidied) = app
        .patch(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{node}"),
            json!({ "at": null }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{tidied}");
    assert!(
        tidied["node"]["at"].is_null(),
        "back to the layout: {tidied}"
    );
}

#[tokio::test]
async fn a_branch_becomes_an_epic_with_its_children_as_tickets() {
    let app = TestApp::spawn().await;
    let (_, made) = app
        .post(
            &app.worker,
            "/v1/mindmaps",
            json!({ "project": "tp", "title": "Payments rebuild" }),
        )
        .await;
    let map = made["mindmap"]["id"].as_str().unwrap().to_string();
    let (_, grown) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "nodes": [{ "text": "API" }] }),
        )
        .await;
    let api = grown["nodes"][0]["id"].as_str().unwrap().to_string();
    app.post(
        &app.worker,
        &format!("/v1/mindmaps/{map}/nodes"),
        json!({ "nodes": [
            { "parent": api, "text": "versioning" },
            { "parent": api, "text": "idempotent retries" },
        ] }),
    )
    .await;

    let (s, promoted) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{api}/promote"),
            json!({ "target": "epic" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{promoted}");
    let epic = promoted["created"]["id"].as_str().unwrap().to_string();
    assert_eq!(promoted["created"]["children"].as_array().unwrap().len(), 2);

    // The epic is a real ticket in the project's initial state, and its children
    // hang off it — so the board needs no special case for a ticket a map made.
    let (s, ticket) = app.get(&app.worker, &format!("/v1/tickets/{epic}")).await;
    assert_eq!(s, StatusCode::OK, "{ticket}");
    assert_eq!(ticket["type"], "epic", "{ticket}");
    assert_eq!(ticket["title"], "API", "{ticket}");
    let child = promoted["created"]["children"][0].as_str().unwrap();
    let (_, child_ticket) = app.get(&app.worker, &format!("/v1/tickets/{child}")).await;
    assert_eq!(child_ticket["parent"], epic, "{child_ticket}");

    // The node STAYS, carrying what it became: the map is the record of how the
    // thinking got there, and this link is what keeps it useful afterwards.
    let (_, whole) = app.get(&app.worker, &format!("/v1/mindmaps/{map}")).await;
    let node = whole["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == json!(api))
        .expect("the promoted node is still on the map");
    assert_eq!(node["promoted"]["kind"], "epic", "{node}");
    assert_eq!(node["promoted"]["id"], json!(epic), "{node}");

    // Twice would make a second epic from one thought, indistinguishable from the
    // first.
    let (s, again) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{api}/promote"),
            json!({ "target": "epic" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{again}");
    assert_eq!(again["code"], "mindmap.already_promoted");
}

#[tokio::test]
async fn a_branch_becomes_an_initiative_seeded_with_its_subtree() {
    let app = TestApp::spawn().await;
    let (_, made) = app
        .post(
            &app.worker,
            "/v1/mindmaps",
            json!({ "project": "tp", "title": "Payments rebuild" }),
        )
        .await;
    let map = made["mindmap"]["id"].as_str().unwrap().to_string();
    let (_, grown) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "nodes": [{ "text": "integrations" }] }),
        )
        .await;
    let node = grown["nodes"][0]["id"].as_str().unwrap().to_string();
    app.post(
        &app.worker,
        &format!("/v1/mindmaps/{map}/nodes"),
        json!({ "parent": node, "text": "Stripe first" }),
    )
    .await;

    let (s, promoted) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{node}/promote"),
            json!({ "target": "initiative" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{promoted}");
    let ini = promoted["created"]["id"].as_str().unwrap().to_string();

    let (s, initiative) = app
        .get(&app.worker, &format!("/v1/initiatives/{ini}"))
        .await;
    assert_eq!(s, StatusCode::OK, "{initiative}");
    assert_eq!(initiative["title"], "integrations", "{initiative}");
    assert_eq!(
        initiative["rollup"]["entries"],
        json!(1),
        "seeded: {initiative}"
    );

    // The subtree arrives as an ENTRY, because an entry carries provenance — and
    // where an idea came from is what a collection is supposed to remember.
    let (_, entries) = app
        .get(&app.worker, &format!("/v1/initiatives/{ini}/entries"))
        .await;
    let entry = &entries["items"][0];
    assert_eq!(entry["source"], format!("mindmap:{map}"), "{entry}");
    assert!(
        entry["text"].as_str().unwrap().contains("Stripe first"),
        "the subtree came with it: {entry}"
    );

    // An unknown target names both, rather than reporting a bad enum.
    let (s, bad) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{node}/promote"),
            json!({ "target": "ticket" }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    assert!(
        bad["message"].as_str().unwrap().contains("initiative"),
        "{bad}"
    );
}

#[tokio::test]
async fn throwing_a_map_away_is_ordinary_and_leaves_what_it_produced() {
    // The clearest statement of what a mindmap is. An initiative is nurtured; a
    // map is scratch — but the work its branches became is work in its own right.
    let app = TestApp::spawn().await;
    let (_, made) = app
        .post(
            &app.worker,
            "/v1/mindmaps",
            json!({ "project": "tp", "title": "Payments rebuild" }),
        )
        .await;
    let map = made["mindmap"]["id"].as_str().unwrap().to_string();
    let (_, grown) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "nodes": [{ "text": "API" }, { "text": "ideas" }] }),
        )
        .await;
    let api = grown["nodes"][0]["id"].as_str().unwrap().to_string();
    let (_, promoted) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{api}/promote"),
            json!({ "target": "epic" }),
        )
        .await;
    let epic = promoted["created"]["id"].as_str().unwrap().to_string();

    let (s, gone) = app
        .delete(&app.worker, &format!("/v1/mindmaps/{map}"))
        .await;
    assert_eq!(s, StatusCode::OK, "{gone}");
    assert_eq!(gone["removed_nodes"], json!(2), "{gone}");

    let (s, missing) = app.get(&app.worker, &format!("/v1/mindmaps/{map}")).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{missing}");

    // The epic outlived the map it came from.
    let (s, ticket) = app.get(&app.worker, &format!("/v1/tickets/{epic}")).await;
    assert_eq!(
        s,
        StatusCode::OK,
        "promoted work survives its map: {ticket}"
    );
}

#[tokio::test]
async fn pruning_a_branch_takes_everything_under_it() {
    let app = TestApp::spawn().await;
    let (_, made) = app
        .post(
            &app.worker,
            "/v1/mindmaps",
            json!({ "project": "tp", "title": "Payments rebuild" }),
        )
        .await;
    let map = made["mindmap"]["id"].as_str().unwrap().to_string();
    let (_, grown) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "nodes": [{ "text": "API" }, { "text": "keep me" }] }),
        )
        .await;
    let api = grown["nodes"][0]["id"].as_str().unwrap().to_string();
    let (_, child) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "parent": api, "text": "versioning" }),
        )
        .await;
    let versioning = child["nodes"][0]["id"].as_str().unwrap().to_string();
    app.post(
        &app.worker,
        &format!("/v1/mindmaps/{map}/nodes"),
        json!({ "parent": versioning, "text": "deeper still" }),
    )
    .await;

    let (s, pruned) = app
        .delete(&app.worker, &format!("/v1/mindmaps/{map}/nodes/{api}"))
        .await;
    assert_eq!(s, StatusCode::OK, "{pruned}");
    assert_eq!(
        pruned["removed"],
        json!(3),
        "the whole branch went: {pruned}"
    );

    let (_, whole) = app.get(&app.worker, &format!("/v1/mindmaps/{map}")).await;
    assert_eq!(whole["total"], json!(1), "{whole}");
    assert_eq!(whole["nodes"][0]["text"], "keep me", "{whole}");
}

#[tokio::test]
async fn mindmaps_list_is_a_bounded_envelope_and_the_archive_gate_applies() {
    let app = TestApp::spawn().await;
    // One map per project, so three maps means three projects. The paging this
    // asserts is still worth asserting — a fleet has many projects — it just
    // cannot be reached by stacking maps under one of them any more.
    for i in 0..3 {
        let project = format!("mm{i}");
        app.create_project_with(&project, common::simple_workflow())
            .await;
        let (s, made) = app
            .post(
                &app.worker,
                "/v1/mindmaps",
                json!({ "project": project, "title": format!("Map {i}") }),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "{made}");
    }

    // A project holds ONE brainstorm, and the refusal names the map it already
    // has — the caller almost always wanted that one.
    let (s, second) = app
        .post(
            &app.worker,
            "/v1/mindmaps",
            json!({ "project": "mm0", "title": "A second map" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{second}");
    assert_eq!(second["code"], "mindmap.project_has_one");
    assert!(
        second["message"].as_str().unwrap().contains("mm-"),
        "the refusal should name the existing map: {second}"
    );
    assert!(
        second["remedy"].as_str().unwrap().contains("promote"),
        "and point at the way out: {second}"
    );

    let (s, page) = app.get(&app.worker, "/v1/mindmaps?limit=2").await;
    assert_eq!(s, StatusCode::OK, "{page}");
    assert_eq!(page["items"].as_array().unwrap().len(), 2, "{page}");
    assert_eq!(page["total"], json!(3), "{page}");
    assert!(
        page["note"].as_str().unwrap().contains("Showing 2 of 3"),
        "{page}"
    );

    // Archiving a project freezes brainstorming under it like every other write —
    // the guard lives in the store, so this route inherits it for free. `tp` has
    // no map of its own, so this reaches the archive gate rather than the cap.
    app.post(&app.admin, "/v1/projects/tp/archive", json!({}))
        .await;
    let (s, frozen) = app
        .post(
            &app.worker,
            "/v1/mindmaps",
            json!({ "project": "tp", "title": "After the freeze" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{frozen}");
    assert_eq!(frozen["code"], "project.archived");

    // Reads keep working, exactly as they do everywhere else.
    let (s, still) = app.get(&app.worker, "/v1/mindmaps").await;
    assert_eq!(s, StatusCode::OK, "{still}");
}

/// "A person approved this case" now says WHICH person, resolvably.
///
/// `human_by` is the free-form actor string the credential carried, which two
/// `human:alice` tokens share and which nothing survives its owner leaving. The
/// verdict now also records the directory person behind the credential — and the
/// case detail resolves every such id once, because "who approved this?" is the
/// question that read exists to answer.
#[tokio::test]
async fn a_human_verdict_records_which_person_gave_it() {
    let app = TestApp::spawn().await;
    let ada = app.add_user("ada", Some("tp"));
    let ada_token = app.mint_as_user("human:ada", &["read", "write", "human"], &ada);

    let id = check(&app, json!({ "title": "Create a claim" })).await;
    file_cases(&app, &id, &["only"]).await;
    let cid = case_ids(&app, &id).await[0].1.clone();

    // An agent's observation first: it must NOT write a human attribution, because
    // that column is the record of a person signing off.
    let (s, agent) = app
        .post(
            &app.worker,
            &format!("/v1/cases/{cid}/verdict"),
            json!({ "verdict": "pass" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{agent}");
    assert!(agent["human"]["user"].is_null(), "{agent}");

    let (s, approved) = app
        .post(
            &ada_token,
            &format!("/v1/cases/{cid}/verdict"),
            json!({ "verdict": "pass", "actor_kind": "human" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{approved}");
    assert_eq!(
        approved["human"]["by"], "human:ada",
        "the actor is unchanged"
    );
    assert_eq!(
        approved["human"]["user"], ada,
        "and now it says who: {approved}"
    );

    // The case detail resolves it, so a reader gets a name without a second request.
    let (s, detail) = app.get(&app.worker, &format!("/v1/cases/{cid}")).await;
    assert_eq!(s, StatusCode::OK, "{detail}");
    assert_eq!(detail["people"][&ada]["handle"], "ada", "{detail}");
    assert_eq!(
        detail["people"][&ada]["label"], "ada the tester",
        "{detail}"
    );

    // The history is the permanent record: both verdicts are there, and the human
    // one carries the person while the agent's carries none.
    let history = detail["history"].as_array().unwrap();
    let human = history
        .iter()
        .find(|v| v["actor_kind"] == "human")
        .expect("the human verdict is in the history");
    assert_eq!(human["user"], ada, "{human}");
    let agent_entry = history
        .iter()
        .find(|v| v["actor_kind"] == "agent")
        .expect("the agent verdict too");
    assert!(agent_entry["user"].is_null(), "{agent_entry}");

    // Disabling her does not erase what she approved — the whole reason the
    // directory has no delete.
    app.post(&app.admin, "/v1/users/ada/disable", json!({}))
        .await;
    let (_, after) = app.get(&app.worker, &format!("/v1/cases/{cid}")).await;
    assert_eq!(after["people"][&ada]["handle"], "ada", "{after}");
    assert_eq!(after["people"][&ada]["disabled"], json!(true), "{after}");
    assert_eq!(after["human"]["user"], ada, "{after}");
}

/// A verdict from a credential bound to nobody still records — it just says so.
///
/// Every verdict taken before the directory existed is in this state, and
/// inventing a person for them would be worse than admitting the gap.
#[tokio::test]
async fn a_verdict_from_an_unbound_credential_names_nobody() {
    let app = TestApp::spawn().await;
    let id = check(&app, json!({ "title": "Create a claim" })).await;
    file_cases(&app, &id, &["only"]).await;
    let cid = case_ids(&app, &id).await[0].1.clone();

    // `app.human` is a human-scoped token with no person behind it.
    let (s, approved) = app
        .post(
            &app.human,
            &format!("/v1/cases/{cid}/verdict"),
            json!({ "verdict": "pass", "actor_kind": "human" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{approved}");
    assert_eq!(approved["human"]["by"], "human:reviewer", "{approved}");
    assert!(approved["human"]["user"].is_null(), "{approved}");

    let (_, detail) = app.get(&app.worker, &format!("/v1/cases/{cid}")).await;
    assert_eq!(
        detail["people"],
        json!({}),
        "nothing here names anybody, and the map says so rather than being absent: {detail}"
    );
}

// ---------------------------------------------------------------------------
// Collaborative documents
//
// The surface exists because the initiative document could not be edited — it
// was reduced from an append-only entry log, "latest view per pane wins", which
// is last-write-wins merge dressed as an audit trail. These routes carry a
// document's FILING only; what it says lives in a Yjs CRDT and moves over the
// WebSocket session.
//
// So the property worth pinning here is mostly negative: no route accepts prose,
// and the ones that exist behave like every other list/patch/archive pair.
// ---------------------------------------------------------------------------

async fn document(app: &TestApp, body: Value) -> Value {
    let (status, b) = app
        .post(&app.admin, "/v1/projects/tp/documents", body)
        .await;
    assert_eq!(status, StatusCode::CREATED, "document create failed: {b}");
    b
}

/// A document is created empty, filed in a folder, and reports how much history
/// it holds — never its text.
#[tokio::test]
async fn a_document_is_filing_and_never_carries_its_prose() {
    let app = TestApp::spawn().await;

    let doc = document(&app, json!({ "title": "Chorleitung" })).await;
    assert_eq!(doc["title"], "Chorleitung");
    assert_eq!(doc["path"], "", "top level by default: {doc}");
    assert_eq!(doc["status"], "draft");
    assert!(doc["initiative"].is_null());
    assert_eq!(
        doc["updates"],
        json!(0),
        "a new document has no history yet: {doc}"
    );
    assert_eq!(doc["bytes"], json!(0), "{doc}");
    assert!(
        doc.get("body").is_none() && doc.get("text").is_none(),
        "the prose must never appear on a JSON route — it is a CRDT, and a text \
         field here would be a second, last-write-wins way to set it: {doc}"
    );

    // And no write route accepts prose either, which is the half that matters:
    // a `body` that were merely ignored would look like it worked.
    let (s, b) = app
        .post(
            &app.admin,
            "/v1/projects/tp/documents",
            json!({ "title": "Smuggled", "body": "hello" }),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::BAD_REQUEST,
        "an unknown 'body' field must be refused, not silently dropped: {b}"
    );

    let id = doc["id"].as_str().unwrap();
    let (s, moved) = app
        .patch(
            &app.admin,
            &format!("/v1/documents/{id}"),
            json!({ "path": "produkt/proben", "status": "review" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{moved}");
    assert_eq!(moved["path"], "produkt/proben");
    assert_eq!(moved["status"], "review");
    assert_eq!(
        moved["version"],
        json!(2),
        "version counts metadata edits: {moved}"
    );
}

/// A folder path has to be renderable as a tree, so the shapes that cannot be
/// are refused rather than stored and drawn wrong.
#[tokio::test]
async fn a_document_path_must_be_a_drawable_folder() {
    let app = TestApp::spawn().await;
    for bad in ["/leading", "trailing/", "a//b", "a/../b", "a/./b"] {
        let (s, b) = app
            .post(
                &app.admin,
                "/v1/projects/tp/documents",
                json!({ "title": "x", "path": bad }),
            )
            .await;
        assert_eq!(
            s,
            StatusCode::UNPROCESSABLE_ENTITY,
            "path '{bad}' must be refused — it would draw a folder nobody can name: {b}"
        );
        assert_eq!(b["code"], "validation.document_path", "{b}");
    }

    // The two legitimate shapes.
    for ok in ["", "produkt", "produkt/billing/v2"] {
        let d = document(&app, json!({ "title": "x", "path": ok })).await;
        assert_eq!(d["path"], ok);
    }
}

/// A document may name the initiative it was distilled from, and that reference
/// is validated on write — the rule `checks.initiative` already set.
///
/// This is the column the eventual migration off the entry-log document surface
/// runs on, so a dangling one would make "which initiatives have been moved
/// across" unanswerable exactly when it is being asked.
#[tokio::test]
async fn a_document_may_name_the_initiative_it_came_from() {
    let app = TestApp::spawn().await;

    let (s, ini) = app
        .post(
            &app.admin,
            "/v1/initiatives",
            json!({ "project": "tp", "title": "Billing" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{ini}");
    let ini_id = ini["id"].as_str().unwrap().to_string();

    let doc = document(&app, json!({ "title": "Billing v2", "initiative": ini_id })).await;
    assert_eq!(doc["initiative"], json!(ini_id));

    let (s, b) = app
        .post(
            &app.admin,
            "/v1/projects/tp/documents",
            json!({ "title": "Nope", "initiative": "ini-doesnotexist" }),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::NOT_FOUND,
        "a reference naming nothing must be refused on write: {b}"
    );

    // `?initiative=none` narrows to documents no initiative claims, the same
    // shape `?epic=none` uses on tickets.
    document(&app, json!({ "title": "Unfiled" })).await;
    let (s, filed) = app
        .get(
            &app.worker,
            &format!("/v1/projects/tp/documents?initiative={ini_id}"),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{filed}");
    assert_eq!(filed["total"], json!(1), "{filed}");
    let (s, unfiled) = app
        .get(&app.worker, "/v1/projects/tp/documents?initiative=none")
        .await;
    assert_eq!(s, StatusCode::OK, "{unfiled}");
    assert_eq!(unfiled["total"], json!(1), "{unfiled}");
    assert_eq!(unfiled["items"][0]["title"], "Unfiled", "{unfiled}");
}

/// Archiving is reversible and leaves the prose untouched — which is what makes
/// bringing one back honest rather than a restore from something lossy.
#[tokio::test]
async fn archiving_a_document_hides_it_and_unarchiving_brings_it_back() {
    let app = TestApp::spawn().await;
    let doc = document(&app, json!({ "title": "Parked" })).await;
    let id = doc["id"].as_str().unwrap();

    let (s, b) = app.delete(&app.admin, &format!("/v1/documents/{id}")).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(!b["archived_at"].is_null(), "{b}");

    let (s, list) = app.get(&app.worker, "/v1/projects/tp/documents").await;
    assert_eq!(s, StatusCode::OK, "{list}");
    assert_eq!(
        list["total"],
        json!(0),
        "archived is hidden by default: {list}"
    );

    let (s, list) = app
        .get(&app.worker, "/v1/projects/tp/documents?archived=include")
        .await;
    assert_eq!(s, StatusCode::OK, "{list}");
    assert_eq!(list["total"], json!(1), "{list}");

    let (s, back) = app
        .post(
            &app.admin,
            &format!("/v1/documents/{id}/unarchive"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{back}");
    assert!(back["archived_at"].is_null(), "{back}");
}

// ---------------------------------------------------------------------------
// The sync socket
//
// These drive a REAL WebSocket against the running server with real Yjs updates
// built by `yrs`, because the properties worth checking here only exist when the
// pieces are talking: does a second peer see the first one's edit, does a
// read-only ticket get refused the write, does the text survive the process
// forgetting the room. Asserting the protocol against hand-written bytes would
// only prove the server agrees with itself.
// ---------------------------------------------------------------------------

mod docsync_support {
    use yrs::encoding::read::{Cursor, Read as _};
    use yrs::encoding::write::Write as _;

    pub const MSG_SYNC: u64 = 0;
    pub const SYNC_STEP1: u64 = 0;
    pub const SYNC_STEP2: u64 = 1;
    pub const SYNC_UPDATE: u64 = 2;

    pub fn sync_message(kind: u64, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.write_var(MSG_SYNC);
        out.write_var(kind);
        out.write_buf(payload);
        out
    }

    /// Pull the update out of a SYNC frame, or `None` for anything else.
    pub fn read_sync(bytes: &[u8]) -> Option<(u64, Vec<u8>)> {
        let mut dec = Cursor::new(bytes);
        let kind: u64 = dec.read_var().ok()?;
        if kind != MSG_SYNC {
            return None;
        }
        let sync_kind: u64 = dec.read_var().ok()?;
        let payload = dec.read_buf().ok()?;
        Some((sync_kind, payload.to_vec()))
    }
}

/// Open a document, mint a ticket, and return `(document id, ws url)`.
async fn document_socket(app: &TestApp, token: &str, title: &str) -> (String, String) {
    let doc = document(app, json!({ "title": title })).await;
    let id = doc["id"].as_str().unwrap().to_string();
    let (s, sess) = app
        .post(token, &format!("/v1/documents/{id}/session"), json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "session mint failed: {sess}");
    let ticket = sess["token"].as_str().unwrap();
    let ws = app.base.replacen("http://", "ws://", 1);
    // The document id is the LAST segment: `y-websocket` composes
    // `serverUrl + "/" + room + "?" + params`, so the route has to be shaped the
    // way that composition produces.
    (id.clone(), format!("{ws}/v1/docsync/{id}?ticket={ticket}"))
}

/// Two peers on one document see each other's edits, and the text is persisted.
///
/// This is the property the whole surface exists for: the initiative document
/// reduced "latest view per pane wins" out of an entry log, which loses whatever
/// was typed while somebody else was writing. Here both edits survive because
/// neither peer ever sends a document — they send operations a CRDT merges.
#[tokio::test]
async fn two_peers_edit_one_document_and_both_edits_survive() {
    use docsync_support::*;
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as WsMessage;
    use yrs::updates::decoder::Decode;
    use yrs::{GetString, Text, Transact};

    let app = TestApp::spawn().await;
    let (id, url) = document_socket(&app, &app.admin, "Konzept").await;

    let (mut a, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("peer A");
    let (mut b, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("peer B");

    // Each peer holds its own replica, exactly as a browser does.
    let doc_a = yrs::Doc::new();
    let text_a = doc_a.get_or_insert_text("prose");
    let doc_b = yrs::Doc::new();
    let text_b = doc_b.get_or_insert_text("prose");

    // A writes.
    let update_a = {
        let mut txn = doc_a.transact_mut();
        text_a.insert(&mut txn, 0, "Wenn jemand absagt, ");
        txn.encode_update_v1()
    };
    a.send(WsMessage::Binary(
        sync_message(SYNC_UPDATE, &update_a).into(),
    ))
    .await
    .unwrap();

    // B should receive it. The server also opens with a STEP1, so read until the
    // update arrives rather than assuming the first frame is it.
    let relayed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let msg = b.next().await.expect("frame").expect("ok");
            if let WsMessage::Binary(bytes) = msg {
                if let Some((kind, payload)) = read_sync(&bytes) {
                    // `len() > 2` skips the empty diff the server sends when it
                    // has nothing the peer is missing — that frame is a correct
                    // answer to our step 1 and not the edit we are waiting for.
                    if (kind == SYNC_UPDATE || kind == SYNC_STEP2) && payload.len() > 2 {
                        return payload;
                    }
                }
            }
        }
    })
    .await
    .expect("peer B must receive peer A's edit");

    {
        let mut txn = doc_b.transact_mut();
        txn.apply_update(yrs::Update::decode_v1(&relayed).unwrap())
            .unwrap();
    }
    assert_eq!(
        text_b.get_string(&doc_b.transact()),
        "Wenn jemand absagt, ",
        "peer B must see peer A's words"
    );

    // Now B writes too, and the CRDT keeps both.
    let update_b = {
        let mut txn = doc_b.transact_mut();
        let len = text_b.get_string(&txn).chars().count() as u32;
        text_b.insert(&mut txn, len, "passiert nichts automatisch.");
        txn.encode_update_v1()
    };
    b.send(WsMessage::Binary(
        sync_message(SYNC_UPDATE, &update_b).into(),
    ))
    .await
    .unwrap();

    // Give the flusher a couple of ticks, then prove it reached SQLite by asking
    // the store what it holds — the document was never sent over a JSON route, so
    // this is the only thing that could have persisted it.
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    let (s, meta) = app.get(&app.worker, &format!("/v1/documents/{id}")).await;
    assert_eq!(s, StatusCode::OK, "{meta}");
    assert!(
        meta["updates"].as_i64().unwrap() >= 1,
        "the flusher must have written the edits: {meta}"
    );
    assert!(meta["bytes"].as_i64().unwrap() > 0, "{meta}");

    // And that what was written rebuilds the text. A third peer joining gets the
    // document from the log, which is the path a page load takes.
    drop(a);
    drop(b);
    let (mut c, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("peer C");
    let doc_c = yrs::Doc::new();
    let text_c = doc_c.get_or_insert_text("prose");
    // Ask for everything by sending an empty state vector.
    let empty_sv = yrs::updates::encoder::Encode::encode_v1(&yrs::StateVector::default());
    c.send(WsMessage::Binary(
        sync_message(SYNC_STEP1, &empty_sv).into(),
    ))
    .await
    .unwrap();

    let restored = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let msg = c.next().await.expect("frame").expect("ok");
            if let WsMessage::Binary(bytes) = msg {
                if let Some((kind, payload)) = read_sync(&bytes) {
                    if kind == SYNC_STEP2 && payload.len() > 2 {
                        return payload;
                    }
                }
            }
        }
    })
    .await
    .expect("a fresh peer must be served the persisted document");

    {
        let mut txn = doc_c.transact_mut();
        txn.apply_update(yrs::Update::decode_v1(&restored).unwrap())
            .unwrap();
    }
    assert_eq!(
        text_c.get_string(&doc_c.transact()),
        "Wenn jemand absagt, passiert nichts automatisch.",
        "both peers' words must survive — this is the whole point of the CRDT"
    );
}

/// A ticket reaches exactly one document, and carries no more than the token
/// that minted it.
///
/// Both halves matter. The ticket travels in a query string, because a browser
/// WebSocket cannot set an Authorization header — so what a leaked one is worth
/// is precisely what these assertions bound.
#[tokio::test]
async fn a_sync_ticket_is_scoped_to_one_document_and_to_its_minter() {
    use docsync_support::*;
    use futures::SinkExt;
    use tokio_tungstenite::tungstenite::Message as WsMessage;
    use yrs::{Text, Transact};

    let app = TestApp::spawn().await;
    let (_first, url) = document_socket(&app, &app.admin, "One").await;

    // A ticket for one document must not open another.
    let other = document(&app, json!({ "title": "Two" })).await;
    let other_id = other["id"].as_str().unwrap();
    let ticket = url.split("ticket=").nth(1).unwrap();
    let ws = app.base.replacen("http://", "ws://", 1);
    let wrong = format!("{ws}/v1/docsync/{other_id}?ticket={ticket}");
    assert!(
        tokio_tungstenite::connect_async(&wrong).await.is_err(),
        "a ticket minted for one document must not open another"
    );

    // No ticket at all is refused too.
    let bare = format!("{ws}/v1/docsync/{other_id}");
    assert!(
        tokio_tungstenite::connect_async(&bare).await.is_err(),
        "the socket must not be reachable without a ticket"
    );

    // A read-only token mints a read-only ticket, and its edits are not applied.
    let (s, readonly_tok) = app
        .post(
            &app.admin,
            "/v1/tokens",
            json!({ "actor": "reader", "scopes": ["read"] }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{readonly_tok}");
    let reader = readonly_tok["token"].as_str().unwrap().to_string();

    let doc = document(&app, json!({ "title": "Read only" })).await;
    let id = doc["id"].as_str().unwrap();
    let (s, sess) = app
        .post(&reader, &format!("/v1/documents/{id}/session"), json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{sess}");
    assert_eq!(
        sess["can_write"],
        json!(false),
        "a read-only token must not mint a writing ticket: {sess}"
    );

    let ro_url = format!(
        "{ws}/v1/docsync/{id}?ticket={}",
        sess["token"].as_str().unwrap()
    );
    let (mut sock, _) = tokio_tungstenite::connect_async(&ro_url)
        .await
        .expect("a reader may still open the document");

    let d = yrs::Doc::new();
    let t = d.get_or_insert_text("prose");
    let update = {
        let mut txn = d.transact_mut();
        t.insert(&mut txn, 0, "smuggled");
        txn.encode_update_v1()
    };
    sock.send(WsMessage::Binary(sync_message(SYNC_UPDATE, &update).into()))
        .await
        .unwrap();
    drop(sock);

    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    let (s, meta) = app.get(&app.worker, &format!("/v1/documents/{id}")).await;
    assert_eq!(s, StatusCode::OK, "{meta}");
    assert_eq!(
        meta["updates"],
        json!(0),
        "a read-only peer's edit must never be persisted: {meta}"
    );
}

// ---------------------------------------------------------------------------
// Mindmaps as a collaborative object.
//
// The nodes stopped being rows and became one Yjs document per map, reusing the
// machinery `/documents` already had. Two properties carry the whole change and
// both are tested below: an existing map survives the conversion, and an agent
// growing a branch shows up on a canvas somebody is already looking at.
// ---------------------------------------------------------------------------

/// Start a map, mint a sync ticket, and return `(mindmap id, ws url)`.
///
/// The mindmap id is the LAST path segment for the same wire reason a document's
/// is: `y-websocket` composes `serverUrl + "/" + room + "?" + params`.
async fn mindmap_socket(app: &TestApp, token: &str, title: &str) -> (String, String) {
    let (s, made) = app
        .post(
            token,
            "/v1/mindmaps",
            json!({ "project": "tp", "title": title }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let id = made["mindmap"]["id"].as_str().unwrap().to_string();
    let (s, sess) = app
        .post(token, &format!("/v1/mindmaps/{id}/session"), json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "session mint failed: {sess}");
    let ticket = sess["token"].as_str().unwrap();
    let ws = app.base.replacen("http://", "ws://", 1);
    (id.clone(), format!("{ws}/v1/docsync/{id}?ticket={ticket}"))
}

/// Attempt a sync handshake that is expected to be refused, and read the
/// refusal.
///
/// The ticket is checked before the upgrade, so a refusal comes back as an
/// ordinary HTTP response carried inside the handshake error rather than as a
/// closed socket — which is what makes it worth asserting the code and not just
/// that the connect failed.
async fn ws_refusal(url: &str) -> (u16, Value) {
    match tokio_tungstenite::connect_async(url).await {
        Ok(_) => panic!("expected {url} to be refused, but the socket opened"),
        Err(tokio_tungstenite::tungstenite::Error::Http(resp)) => {
            let status = resp.status().as_u16();
            let body = resp.body().clone().unwrap_or_default();
            (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
        }
        Err(e) => panic!("expected an HTTP refusal from {url}, got: {e}"),
    }
}

/// Every node title in a peer's own replica, sorted so the assertion does not
/// depend on map iteration order.
///
/// A peer reads the CRDT the way the browser does — `nodes` is a `Y.Map` of
/// `Y.Map`, and `title` inside one is a `Y.Text` because two people really do
/// type into the same node.
fn peer_node_titles(doc: &yrs::Doc) -> Vec<String> {
    use yrs::{Any, GetString, Map, Out, Transact};
    let nodes = doc.get_or_insert_map("nodes");
    let txn = doc.transact();
    let mut out = Vec::new();
    for (_, value) in nodes.iter(&txn) {
        let Out::YMap(entry) = value else { continue };
        match entry.get(&txn, "title") {
            Some(Out::YText(t)) => out.push(t.get_string(&txn)),
            Some(Out::Any(Any::String(s))) => out.push(s.to_string()),
            _ => {}
        }
    }
    out.sort();
    out
}

/// Build one node the way a browser peer does, and return the update to send.
fn peer_node_update(doc: &yrs::Doc, id: &str, title: &str, order: &str) -> Vec<u8> {
    use yrs::types::text::TextPrelim;
    use yrs::{Map, MapPrelim, Transact};
    let nodes = doc.get_or_insert_map("nodes");
    let mut txn = doc.transact_mut();
    let entry = nodes.insert(&mut txn, id.to_string(), MapPrelim::default());
    entry.insert(&mut txn, "title", TextPrelim::new(title));
    entry.insert(&mut txn, "notes", TextPrelim::new(""));
    entry.insert(&mut txn, "order", order.to_string());
    txn.encode_update_v1()
}

/// The plan's history: what happened, and WHO did it.
///
/// The CRDT update log rebuilds the text. It cannot answer "who changed 2.1,
/// and has anybody agreed with it since" — it is written per flush, merged, and
/// rewritten by compaction. This is the record of acts somebody would name, and
/// the reason the document view is worth having.
#[tokio::test]
async fn the_plan_keeps_a_trace_of_what_people_did_to_it() {
    let app = TestApp::spawn().await;
    let ada = app.add_user("ada", Some("tp"));
    let ada_token = app.mint_as_user("human:ada", &["read", "write", "human"], &ada);

    let (_, made) = app
        .post(
            &ada_token,
            "/v1/mindmaps",
            json!({ "project": "tp", "title": "Payments rebuild" }),
        )
        .await;
    let map = made["mindmap"]["id"].as_str().unwrap().to_string();

    let (s, out) = app
        .post(
            &ada_token,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "text": "API" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{out}");
    let node = out["nodes"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(
        out["nodes"][0]["created_by_user"],
        json!(ada),
        "the node records the PERSON, not only the actor string: {out}"
    );

    // Rename and write into it: two different acts, two entries.
    app.patch(
        &ada_token,
        &format!("/v1/mindmaps/{map}/nodes/{node}"),
        json!({ "title": "The API", "notes": "v1 forever, additive only." }),
    )
    .await;

    let (s, trace) = app
        .get(&ada_token, &format!("/v1/mindmaps/{map}/trace"))
        .await;
    assert_eq!(s, StatusCode::OK, "{trace}");
    let kinds: Vec<&str> = trace["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"authored"), "{trace}");
    assert!(kinds.contains(&"renamed"), "{trace}");
    assert!(kinds.contains(&"edited"), "{trace}");
    for entry in trace["items"].as_array().unwrap() {
        assert_eq!(
            entry["user"],
            json!(ada),
            "every act names the person behind the credential: {entry}"
        );
    }

    // Nothing is confirmed until somebody says so.
    let (_, read) = app.get(&ada_token, &format!("/v1/mindmaps/{map}")).await;
    assert_eq!(read["standing"][&node]["confirmed"], json!(false), "{read}");

    let (s, ok) = app
        .post(
            &ada_token,
            &format!("/v1/mindmaps/{map}/trace"),
            json!({ "node": node, "kind": "reviewed" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{ok}");

    let (_, read) = app.get(&ada_token, &format!("/v1/mindmaps/{map}")).await;
    assert_eq!(read["standing"][&node]["confirmed"], json!(true), "{read}");

    // And a confirmation is only good until the next change — which is exactly
    // what a stored boolean could not express.
    app.patch(
        &ada_token,
        &format!("/v1/mindmaps/{map}/nodes/{node}"),
        json!({ "notes": "Changed my mind: dated versions." }),
    )
    .await;
    let (_, read) = app.get(&ada_token, &format!("/v1/mindmaps/{map}")).await;
    assert_eq!(
        read["standing"][&node]["confirmed"],
        json!(false),
        "an edit after a review un-confirms the section: {read}"
    );

    // History keeps what the section SAID, which is what a diff needs two of.
    let (_, trace) = app
        .get(&ada_token, &format!("/v1/mindmaps/{map}/trace?node={node}"))
        .await;
    let said: Vec<&str> = trace["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["text"].as_str())
        .collect();
    assert!(
        said.iter().any(|t| t.contains("Changed my mind")),
        "the newest edit kept what it left behind: {trace}"
    );
    assert!(
        said.iter().any(|t| t.contains("v1 forever")),
        "and the one before it kept what IT said — two sides, which is a diff: {trace}"
    );
    let moved: Vec<&Value> = trace["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == json!("moved"))
        .collect();
    assert!(
        moved.iter().all(|e| e["text"].is_null()),
        "an act that did not touch the prose keeps no copy of it"
    );

    // A caller may not claim to have done what the server records itself.
    let (s, refused) = app
        .post(
            &ada_token,
            &format!("/v1/mindmaps/{map}/trace"),
            json!({ "node": node, "kind": "moved" }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
    assert_eq!(refused["code"], "validation.trace_kind");
}

/// An API write that answered has to have been written down.
///
/// The map's replica lives in memory and a debounced flusher writes it out every
/// couple of seconds — which is right for somebody typing in a browser, where
/// batching is the difference between a smooth canvas and every keystroke queued
/// behind the process-wide write mutex. A request is not typing: it makes one
/// change and then says it did.
///
/// Without that distinction the room can be torn down — the last peer leaves
/// when the handler returns — while its edits are still only in memory, and the
/// next request rebuilds the replica from a log that never received them. It
/// showed up first as two unrelated tests failing about one run in three, which
/// is exactly how this class of bug announces itself.
#[tokio::test]
async fn a_mindmap_write_is_on_disk_before_the_request_answers() {
    let app = TestApp::spawn().await;
    let (_, made) = app
        .post(
            &app.worker,
            "/v1/mindmaps",
            json!({ "project": "tp", "title": "Durability" }),
        )
        .await;
    let map = made["mindmap"]["id"].as_str().unwrap().to_string();

    let rows = |map: &str| -> i64 {
        let conn = rusqlite::Connection::open(app.db_path()).expect("open db");
        conn.query_row(
            "SELECT COUNT(*) FROM crdt_updates WHERE object_id = ?1",
            rusqlite::params![map],
            |r| r.get(0),
        )
        .expect("count")
    };

    assert_eq!(rows(&map), 0, "a map with no nodes has written nothing yet");

    let (s, out) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "text": "written down before you were told" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{out}");
    // No sleep. The point is that this is true the instant the response lands.
    assert!(
        rows(&map) > 0,
        "the node reached the log before the 201 did"
    );

    let node = out["nodes"][0]["id"].as_str().unwrap().to_string();
    let (s, _) = app
        .delete(&app.worker, &format!("/v1/mindmaps/{map}/nodes/{node}"))
        .await;
    assert_eq!(s, StatusCode::OK);
    let after_delete = rows(&map);

    let (s, _) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "text": "and so did this one" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    assert!(
        rows(&map) > after_delete,
        "every mutation persists, not just the first"
    );
}

/// The migration that moves every document's prose into the shared log.
///
/// This is the one test whose absence would have been expensive. The mindmap
/// conversion had a test from the start; the document log — which holds all the
/// prose anybody has written, and whose old table is DROPPED — had none, and the
/// first version of that migration was not atomic. An interrupted run left both
/// tables present, which the guard read as "already done", and every document
/// opened blank with the real bytes stranded in a table nothing read. The server
/// started fine. Nothing said a word.
///
/// So this builds a genuine pre-upgrade database by hand and opens it.
#[test]
fn a_document_written_before_the_shared_log_survives_the_move_into_it() {
    use rusqlite::params;
    use takomo::store::Store;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("legacy.db");

    // The old world: the two document-only tables, exactly as they were shaped
    // before the log was widened.
    {
        let conn = rusqlite::Connection::open(&path).expect("open");
        conn.execute_batch(
            "CREATE TABLE projects (id TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '');
             -- `crdt_sessions` carries a foreign key to `users`, and it is
             -- created before the schema batch runs, so the table it points at
             -- has to exist. On a real pre-upgrade database it always does.
             CREATE TABLE users (id TEXT PRIMARY KEY);
             CREATE TABLE documents (
               id TEXT PRIMARY KEY, project TEXT NOT NULL, title TEXT NOT NULL,
               path TEXT NOT NULL DEFAULT '', status TEXT NOT NULL DEFAULT 'draft',
               initiative TEXT, metadata TEXT NOT NULL DEFAULT 'null',
               version INTEGER NOT NULL DEFAULT 1, created_by TEXT NOT NULL,
               created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, archived_at INTEGER);
             CREATE TABLE doc_updates (
               seq INTEGER PRIMARY KEY AUTOINCREMENT,
               document TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
               blob BLOB NOT NULL, bytes INTEGER NOT NULL,
               created_by TEXT NOT NULL, created_at INTEGER NOT NULL);
             CREATE TABLE doc_sessions (
               id TEXT PRIMARY KEY, token_hash TEXT NOT NULL UNIQUE,
               document TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
               project TEXT NOT NULL, actor TEXT NOT NULL, \"user\" TEXT,
               display TEXT NOT NULL, can_write INTEGER NOT NULL,
               expires_at INTEGER NOT NULL, created_at INTEGER NOT NULL, revoked_at INTEGER);",
        )
        .expect("legacy schema");
        conn.execute("INSERT INTO projects (id, name) VALUES ('tp', 'tp')", [])
            .expect("project");
        conn.execute(
            "INSERT INTO documents (id, project, title, created_by, created_at, updated_at)
             VALUES ('doc-legacy01', 'tp', 'Written before the move', 'someone', 1, 1)",
            [],
        )
        .expect("document");

        // Three flushes, in order. Replay order is the whole point: a Yjs log
        // replayed out of order rebuilds a different document.
        for (seq, body) in [(1i64, "first"), (2, "second"), (3, "third")] {
            conn.execute(
                "INSERT INTO doc_updates (seq, document, blob, bytes, created_by, created_at)
                 VALUES (?1, 'doc-legacy01', ?2, ?3, 'someone', ?1)",
                params![seq, body.as_bytes(), body.len() as i64],
            )
            .expect("update");
        }
        conn.execute(
            "INSERT INTO doc_sessions (id, token_hash, document, project, actor, display,
                                       can_write, expires_at, created_at)
             VALUES ('ds-legacy01', 'hash-legacy', 'doc-legacy01', 'tp', 'someone', 'Someone', 1, 9999999999999, 1)",
            [],
        )
        .expect("session");
    }

    // Opening the store is what runs the migration.
    let store = Store::open(&path).expect("open the pre-upgrade database");

    let updates = store
        .load_collab_updates("doc-legacy01")
        .expect("the document's history");
    assert_eq!(
        updates,
        vec![b"first".to_vec(), b"second".to_vec(), b"third".to_vec()],
        "every flush must survive, in the order it was written"
    );

    {
        let conn = rusqlite::Connection::open(&path).expect("reopen");
        let doc_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM crdt_updates WHERE object_kind = 'document'",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(doc_rows, 3, "and land as document rows in the shared log");

        let old_gone: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('doc_updates', 'doc_sessions')",
                [],
                |r| r.get(0),
            )
            .expect("tables");
        assert_eq!(old_gone, 0, "the old tables are gone, not left to drift");

        let session_object: String = conn
            .query_row(
                "SELECT object_id FROM crdt_sessions WHERE id = 'ds-legacy01'",
                [],
                |r| r.get(0),
            )
            .expect("the ticket moved too");
        assert_eq!(session_object, "doc-legacy01");
    }

    // Opening again must be a no-op rather than a second copy.
    drop(store);
    let store = Store::open(&path).expect("reopen the store");
    assert_eq!(
        store
            .load_collab_updates("doc-legacy01")
            .expect("history")
            .len(),
        3,
        "a second open must not duplicate the log"
    );
}

/// An interrupted move must not read as a finished one.
///
/// This is the exact state a killed process used to leave behind: the new table
/// created, nothing copied into it, the old table still holding every byte. The
/// old guard saw the new table and skipped, and every document came back empty.
/// Now the whole move is one transaction, so this state cannot be produced — and
/// if it is ever found anyway, it must be refused loudly rather than served.
#[test]
fn a_half_finished_log_migration_is_refused_rather_than_served_empty() {
    use rusqlite::params;
    use takomo::store::Store;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("interrupted.db");
    {
        let conn = rusqlite::Connection::open(&path).expect("open");
        conn.execute_batch(
            "CREATE TABLE projects (id TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '');
             CREATE TABLE documents (
               id TEXT PRIMARY KEY, project TEXT NOT NULL, title TEXT NOT NULL,
               path TEXT NOT NULL DEFAULT '', status TEXT NOT NULL DEFAULT 'draft',
               initiative TEXT, metadata TEXT NOT NULL DEFAULT 'null',
               version INTEGER NOT NULL DEFAULT 1, created_by TEXT NOT NULL,
               created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, archived_at INTEGER);
             CREATE TABLE doc_updates (
               seq INTEGER PRIMARY KEY AUTOINCREMENT, document TEXT NOT NULL,
               blob BLOB NOT NULL, bytes INTEGER NOT NULL,
               created_by TEXT NOT NULL, created_at INTEGER NOT NULL);
             CREATE TABLE crdt_updates (
               seq INTEGER PRIMARY KEY AUTOINCREMENT, object_kind TEXT NOT NULL,
               object_id TEXT NOT NULL, blob BLOB NOT NULL, bytes INTEGER NOT NULL,
               created_by TEXT NOT NULL, created_at INTEGER NOT NULL);",
        )
        .expect("schema");
        conn.execute("INSERT INTO projects (id, name) VALUES ('tp', 'tp')", [])
            .expect("project");
        conn.execute(
            "INSERT INTO documents (id, project, title, created_by, created_at, updated_at)
             VALUES ('doc-halfway01', 'tp', 'Halfway', 'someone', 1, 1)",
            [],
        )
        .expect("document");
        conn.execute(
            "INSERT INTO doc_updates (document, blob, bytes, created_by, created_at)
             VALUES ('doc-halfway01', ?1, 5, 'someone', 1)",
            params![b"first".to_vec()],
        )
        .expect("update");
        // The copy landed for this one, and then the process died.
        conn.execute(
            "INSERT INTO crdt_updates (object_kind, object_id, blob, bytes, created_by, created_at)
             VALUES ('document', 'doc-halfway01', ?1, 5, 'someone', 1)",
            params![b"first".to_vec()],
        )
        .expect("copied row");
    }

    let opened = Store::open(&path);
    assert!(
        opened.is_err(),
        "a database in a state the migration cannot produce must be refused, not silently served"
    );
}

/// A map that existed as rows before the CRDT converts, whole, on the first open.
///
/// This is the test that matters most on this change. Everything else here is a
/// new capability that simply would not work; a bad conversion is a *silent*
/// failure — the map opens, it just is not the map somebody left — so every fact
/// the old rows carried is asserted individually rather than by a node count.
#[tokio::test]
async fn a_mindmap_stored_as_rows_before_the_crdt_survives_the_conversion() {
    use rusqlite::params;

    let app = TestApp::spawn().await;
    // The row is created through the API so its project and metadata are real;
    // no node has been added, so the map has no CRDT log yet — which is exactly
    // the state every pre-existing map is in when the new binary first opens it.
    let (_, made) = app
        .post(
            &app.worker,
            "/v1/mindmaps",
            json!({ "project": "tp", "title": "Payments rebuild" }),
        )
        .await;
    let map = made["mindmap"]["id"].as_str().unwrap().to_string();

    // The old shape, written by hand: gapped positions, hand placement on one
    // node, a promotion link on another. `ORDER BY position, created_at, id` is
    // what the reader used, so the rows are deliberately inserted out of that
    // order to prove the conversion sorts rather than trusts insertion order.
    {
        /// One row of the old table, named rather than positional: eight
        /// anonymous tuple fields is a shape nobody can read a fixture out of.
        struct LegacyNode {
            id: &'static str,
            parent: Option<&'static str>,
            text: &'static str,
            position: i64,
            x: Option<f64>,
            y: Option<f64>,
            promoted_kind: Option<&'static str>,
            promoted_id: Option<&'static str>,
        }

        let conn = rusqlite::Connection::open(app.db_path()).expect("open db");
        let rows = [
            LegacyNode {
                id: "mn-legacy02",
                parent: None,
                text: "integrations",
                position: 2000,
                x: Some(12.5),
                y: Some(-4.0),
                promoted_kind: None,
                promoted_id: None,
            },
            LegacyNode {
                id: "mn-legacy01",
                parent: None,
                text: "API",
                position: 1000,
                x: None,
                y: None,
                promoted_kind: None,
                promoted_id: None,
            },
            LegacyNode {
                id: "mn-legacy04",
                parent: Some("mn-legacy01"),
                text: "auth",
                position: 2000,
                x: None,
                y: None,
                promoted_kind: Some("initiative"),
                promoted_id: Some("ini-legacy"),
            },
            LegacyNode {
                id: "mn-legacy03",
                parent: Some("mn-legacy01"),
                text: "versioning?",
                position: 1000,
                x: None,
                y: None,
                promoted_kind: None,
                promoted_id: None,
            },
        ];
        for row in rows {
            conn.execute(
                "INSERT INTO mindmap_nodes (id, mindmap, parent, text, position, x, y, \
                 promoted_kind, promoted_id, created_by, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'human:ada', 1700000000000, 1700000001000)",
                params![
                    row.id,
                    map,
                    row.parent,
                    row.text,
                    row.position,
                    row.x,
                    row.y,
                    row.promoted_kind,
                    row.promoted_id,
                ],
            )
            .expect("insert legacy node");
        }
        // Nothing has converted it yet.
        let logged: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM crdt_updates WHERE object_id = ?1",
                params![map],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(logged, 0, "the fixture must be a genuinely unconverted map");
    }

    // Opening the store is what runs the conversion — `Store::open` calls
    // `adopt_legacy_nodes`, so an operator upgrading the binary pays for it once
    // at startup and never has to run anything.
    let _converted = app.open_store();

    let (s, whole) = app.get(&app.worker, &format!("/v1/mindmaps/{map}")).await;
    assert_eq!(s, StatusCode::OK, "{whole}");
    let nodes = whole["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 4, "every node survives: {whole}");
    assert_eq!(
        whole["mindmap"]["nodes"],
        json!(4),
        "the map's node count is exact on this read: {whole}"
    );

    // Tree order: parents before their children, siblings in the order the old
    // gapped positions put them. A map somebody had arranged by hand comes back
    // looking exactly as it was left, which is the whole promise of the
    // conversion.
    let order: Vec<&str> = nodes.iter().map(|n| n["text"].as_str().unwrap()).collect();
    assert_eq!(
        order,
        vec!["API", "versioning?", "auth", "integrations"],
        "the old sort survives: {whole}"
    );

    let by_id = |id: &str| -> Value {
        nodes
            .iter()
            .find(|n| n["id"] == json!(id))
            .unwrap_or_else(|| panic!("{id} missing from {whole}"))
            .clone()
    };

    // Parentage, and `position` as it now reads: a rank among SIBLINGS, so both
    // rings start at 0 rather than continuing a global sequence.
    let api = by_id("mn-legacy01");
    assert_eq!(api["parent"], Value::Null, "{api}");
    assert_eq!(api["position"], json!(0), "{api}");
    assert_eq!(by_id("mn-legacy02")["position"], json!(1));
    assert_eq!(by_id("mn-legacy03")["parent"], json!("mn-legacy01"));
    assert_eq!(by_id("mn-legacy03")["position"], json!(0));
    assert_eq!(by_id("mn-legacy04")["position"], json!(1));

    // Hand placement carries across. It is a fact somebody established by
    // dragging a node and it is not recoverable if dropped.
    assert_eq!(
        by_id("mn-legacy02")["at"],
        json!({ "x": 12.5, "y": -4.0 }),
        "placement survives"
    );
    assert_eq!(
        api["at"],
        Value::Null,
        "a node nobody dragged stays unplaced"
    );

    // So does the link to what a branch became — the reason a map keeps earning
    // its place after the brainstorming is over.
    assert_eq!(
        by_id("mn-legacy04")["promoted"],
        json!({ "kind": "initiative", "id": "ini-legacy" }),
        "the promotion link survives"
    );
    assert_eq!(api["promoted"], Value::Null);

    // Authorship, and the defaults for everything the old model had no column
    // for. `origin` is `human` because nobody can honestly say otherwise about a
    // node that predates the question.
    assert_eq!(api["created_by"], json!("human:ada"), "{api}");
    assert_eq!(api["notes"], json!(""), "{api}");
    assert_eq!(api["origin"], json!("human"), "{api}");
    assert_eq!(api["kind"], json!("thought"), "{api}");
    assert_eq!(api["title"], api["text"], "both names for one field: {api}");
    assert_eq!(whole["relationships"], json!([]), "{whole}");

    // The outline — the shape a model reads — is rebuilt from the document, not
    // from the rows that are still sitting there.
    let (s, outline) = app
        .get(&app.worker, &format!("/v1/mindmaps/{map}/outline"))
        .await;
    assert_eq!(s, StatusCode::OK, "{outline}");
    assert!(
        outline["outline"]
            .as_str()
            .unwrap()
            .contains("- API\n  - versioning?\n  - auth\n- integrations"),
        "the converted tree reads back in shape: {outline}"
    );

    // And it converts exactly once. A second open must not append a second copy
    // of the map on top of itself — the guard is "no log yet", so this is the
    // assertion that the guard is the right one.
    let _again = app.open_store();
    let (s, twice) = app.get(&app.worker, &format!("/v1/mindmaps/{map}")).await;
    assert_eq!(s, StatusCode::OK, "{twice}");
    assert_eq!(
        twice["nodes"].as_array().unwrap().len(),
        4,
        "converting twice must not duplicate the map: {twice}"
    );
}

/// An agent growing a branch appears on a canvas somebody is already looking at.
///
/// This is the point of the whole change. Before it, the API wrote rows and the
/// browser found out on its next reload; now the REST write lands on the same
/// replica the socket peers hold, so there are not two copies to reconcile.
#[tokio::test]
async fn a_node_added_over_the_api_reaches_a_connected_canvas_without_a_reload() {
    use docsync_support::*;
    use futures::StreamExt;
    use tokio_tungstenite::tungstenite::Message as WsMessage;
    use yrs::updates::decoder::Decode;
    use yrs::Transact;

    let app = TestApp::spawn().await;
    let (map, url) = mindmap_socket(&app, &app.admin, "Payments rebuild").await;

    let (mut peer, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("a canvas connects");

    // The peer holds its own replica, exactly as the browser does.
    let mine = yrs::Doc::new();
    let _ = mine.get_or_insert_map("nodes");

    // The agent's write goes through the ordinary REST route — no socket, no
    // awareness of who is watching.
    let (s, grown) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "text": "settlement window" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{grown}");

    // The canvas must learn about it over the socket it already has open.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let msg = peer.next().await.expect("frame").expect("ok");
            let WsMessage::Binary(bytes) = msg else {
                continue;
            };
            let Some((kind, payload)) = read_sync(&bytes) else {
                continue;
            };
            // Skip the server's opening STEP1 (a state vector, not an update)
            // and the empty diff it answers our own STEP1 with.
            if (kind != SYNC_UPDATE && kind != SYNC_STEP2) || payload.len() <= 2 {
                continue;
            }
            let mut txn = mine.transact_mut();
            txn.apply_update(yrs::Update::decode_v1(&payload).unwrap())
                .unwrap();
            drop(txn);
            if peer_node_titles(&mine) == vec!["settlement window".to_string()] {
                return;
            }
        }
    })
    .await
    .expect("the agent's node must reach the open canvas without a reload");
}

/// Two people growing one map at the same time both keep their branch.
///
/// The mirror of `two_peers_edit_one_document_and_both_edits_survive`, and the
/// reason a mindmap became a CRDT at all: the old store wrote rows, so two
/// people adding to one map raced, and the loser found out by not finding their
/// node. Neither peer here ever sends a map — they send operations that merge.
#[tokio::test]
async fn two_peers_grow_one_mindmap_and_both_branches_survive() {
    use docsync_support::*;
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as WsMessage;
    use yrs::updates::decoder::Decode;
    use yrs::Transact;

    let app = TestApp::spawn().await;
    let (map, url) = mindmap_socket(&app, &app.admin, "Payments rebuild").await;

    let (mut a, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("peer A");
    let (mut b, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("peer B");

    let doc_a = yrs::Doc::new();
    let doc_b = yrs::Doc::new();
    let _ = doc_b.get_or_insert_map("nodes");

    let update_a = peer_node_update(&doc_a, "mn-peera001", "Rückerstattungen", "a0");
    a.send(WsMessage::Binary(
        sync_message(SYNC_UPDATE, &update_a).into(),
    ))
    .await
    .unwrap();

    // B sees A's branch. The server also opens with a STEP1, so read until the
    // update arrives rather than assuming the first frame is it.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let msg = b.next().await.expect("frame").expect("ok");
            let WsMessage::Binary(bytes) = msg else {
                continue;
            };
            let Some((kind, payload)) = read_sync(&bytes) else {
                continue;
            };
            if (kind != SYNC_UPDATE && kind != SYNC_STEP2) || payload.len() <= 2 {
                continue;
            }
            let mut txn = doc_b.transact_mut();
            txn.apply_update(yrs::Update::decode_v1(&payload).unwrap())
                .unwrap();
            drop(txn);
            if !peer_node_titles(&doc_b).is_empty() {
                return;
            }
        }
    })
    .await
    .expect("peer B must see peer A's branch");
    assert_eq!(
        peer_node_titles(&doc_b),
        vec!["Rückerstattungen".to_string()],
        "peer B must see peer A's words"
    );

    // Now B grows its own, on top of what it just merged.
    let update_b = peer_node_update(&doc_b, "mn-peerb001", "Streitfälle", "a1");
    b.send(WsMessage::Binary(
        sync_message(SYNC_UPDATE, &update_b).into(),
    ))
    .await
    .unwrap();

    // Both peers leave, so the room flushes and is dropped; the read below then
    // rebuilds the map from the log, which is the path a page load takes.
    drop(a);
    drop(b);
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    let (s, whole) = app.get(&app.worker, &format!("/v1/mindmaps/{map}")).await;
    assert_eq!(s, StatusCode::OK, "{whole}");
    let titles: Vec<&str> = whole["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["text"].as_str().unwrap())
        .collect();
    assert!(
        titles.contains(&"Rückerstattungen") && titles.contains(&"Streitfälle"),
        "both peers' branches must survive — this is the whole point of the CRDT: {whole}"
    );
}

/// A sync ticket reaches one object, and a map's ticket is not a document's.
///
/// `tkd_` was widened from "document session" to "collab session" rather than
/// given a sixth prefix, so this is the assertion that widening the credential
/// did not widen what one of them reaches. The ticket travels in a query string
/// — a browser WebSocket cannot set an Authorization header — so what a leaked
/// one is worth is precisely what these refusals bound.
#[tokio::test]
async fn a_sync_ticket_for_a_map_cannot_open_a_document_and_the_reverse() {
    let app = TestApp::spawn().await;
    let (map, map_url) = mindmap_socket(&app, &app.admin, "Payments rebuild").await;
    let (doc, doc_url) = document_socket(&app, &app.admin, "Konzept").await;
    let map_ticket = map_url.split("ticket=").nth(1).unwrap().to_string();
    let doc_ticket = doc_url.split("ticket=").nth(1).unwrap().to_string();

    let ws = app.base.replacen("http://", "ws://", 1);

    // A refused handshake answers as ordinary HTTP — the upgrade has not
    // happened yet — which is also what a browser surfaces most usefully on a
    // failed connect. So the refusal carries a code, not just a closed socket.
    let (status, body) = ws_refusal(&format!("{ws}/v1/docsync/{doc}?ticket={map_ticket}")).await;
    assert_eq!(status, StatusCode::FORBIDDEN.as_u16(), "{body}");
    assert_eq!(
        body["code"], "document.session_wrong_document",
        "a map's ticket must not open a document: {body}"
    );

    let (status, body) = ws_refusal(&format!("{ws}/v1/docsync/{map}?ticket={doc_ticket}")).await;
    assert_eq!(status, StatusCode::FORBIDDEN.as_u16(), "{body}");
    assert_eq!(
        body["code"], "mindmap.session_wrong_mindmap",
        "a document's ticket must not open a map: {body}"
    );

    // And the map's socket is not reachable without a ticket at all.
    let (status, body) = ws_refusal(&format!("{ws}/v1/docsync/{map}")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED.as_u16(), "{body}");
    assert_eq!(body["code"], "mindmap.session_missing", "{body}");

    // The mint routes are scoped the same way, and check the kind against the id
    // rather than trusting the route: minting is where a caller would otherwise
    // be handed a ticket for something they did not ask for.
    let (s, wrong) = app
        .post(
            &app.admin,
            &format!("/v1/documents/{map}/session"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{wrong}");
    assert_eq!(wrong["code"], "notfound.document", "{wrong}");
    let (s, wrong) = app
        .post(
            &app.admin,
            &format!("/v1/mindmaps/{doc}/session"),
            json!({}),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{wrong}");
    assert_eq!(wrong["code"], "notfound.mindmap", "{wrong}");

    // A read-only token mints a read-only ticket for a map, exactly as for a
    // document — the ticket carries no more than the token that asked for it.
    let reader = app.mint("reader", &["read"], None);
    let (s, sess) = app
        .post(&reader, &format!("/v1/mindmaps/{map}/session"), json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{sess}");
    assert_eq!(sess["can_write"], json!(false), "{sess}");
    assert_eq!(sess["kind"], json!("mindmap"), "{sess}");
    assert_eq!(sess["object"], json!(map), "{sess}");
    assert_eq!(
        sess["mindmap"], sess["object"],
        "the kind-specific key stays, so a client written against /documents keeps working: {sess}"
    );
    assert_eq!(
        sess["room"],
        json!(map),
        "the room is the bare id — y-websocket appends it to `url`: {sess}"
    );
}

/// A relationship is an edge outside the hierarchy, and it dies with its ends.
///
/// The tree answers "what is this part of"; this answers everything else. It is
/// one mechanism instead of three special cases, which is why it is a separate
/// collection rather than more fields on a node.
#[tokio::test]
async fn a_relationship_links_two_nodes_outside_the_hierarchy_and_goes_with_them() {
    let app = TestApp::spawn().await;
    let (_, made) = app
        .post(
            &app.worker,
            "/v1/mindmaps",
            json!({ "project": "tp", "title": "Payments rebuild" }),
        )
        .await;
    let map = made["mindmap"]["id"].as_str().unwrap().to_string();
    let (_, grown) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "nodes": [
                { "text": "checkout" },
                { "text": "refunds" },
                { "text": "who owns the refund window?", "kind": "question" },
            ] }),
        )
        .await;
    let ids: Vec<String> = grown["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].as_str().unwrap().to_string())
        .collect();
    let (checkout, refunds, question) = (&ids[0], &ids[1], &ids[2]);

    let (s, made) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/relationships"),
            json!({ "from": question, "to": refunds, "label": "asks about" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let rel = made["relationship"]["id"].as_str().unwrap().to_string();
    assert!(rel.starts_with("mr-"), "{made}");
    assert_eq!(made["relationship"]["label"], json!("asks about"), "{made}");

    // It comes back with the map, because a canvas has to draw the cross-links
    // in the same pass it draws the tree.
    let (s, whole) = app.get(&app.worker, &format!("/v1/mindmaps/{map}")).await;
    assert_eq!(s, StatusCode::OK, "{whole}");
    assert_eq!(
        whole["relationships"],
        json!([{ "id": rel, "from": question, "to": refunds, "label": "asks about" }]),
        "{whole}"
    );
    // And it does not pretend to be part of the tree.
    assert_eq!(
        whole["total"],
        json!(3),
        "a relationship is not a node: {whole}"
    );

    // An edge to nothing is refused rather than stored and hidden on read: the
    // caller named a node, and being told which one is missing is the answer.
    let (s, refused) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/relationships"),
            json!({ "from": question, "to": "mn-nosuchnode" }),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{refused}");
    assert_eq!(refused["code"], "notfound.mindmap_node", "{refused}");

    // A self-edge says nothing, and drawing one only makes the canvas harder to
    // read.
    let (s, refused) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/relationships"),
            json!({ "from": question, "to": question }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
    assert_eq!(
        refused["code"], "validation.mindmap_relationship",
        "{refused}"
    );

    // Deleting one takes only the edge; both nodes stay where they are.
    let (s, second) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/relationships"),
            json!({ "from": checkout, "to": refunds }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{second}");
    let second_id = second["relationship"]["id"].as_str().unwrap().to_string();
    let (s, gone) = app
        .delete(
            &app.worker,
            &format!("/v1/mindmaps/{map}/relationships/{second_id}"),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{gone}");
    assert_eq!(gone["ok"], json!(true), "{gone}");
    let (s, gone) = app
        .delete(
            &app.worker,
            &format!("/v1/mindmaps/{map}/relationships/{second_id}"),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{gone}");
    assert_eq!(gone["code"], "notfound.mindmap_relationship", "{gone}");
    let (_, whole) = app.get(&app.worker, &format!("/v1/mindmaps/{map}")).await;
    assert_eq!(
        whole["total"],
        json!(3),
        "deleting an edge keeps both nodes: {whole}"
    );

    // Pruning a node takes its cross-links with it. A dangling edge is not a
    // node and there is nothing to keep.
    let (s, pruned) = app
        .delete(&app.worker, &format!("/v1/mindmaps/{map}/nodes/{refunds}"))
        .await;
    assert_eq!(s, StatusCode::OK, "{pruned}");
    let (_, whole) = app.get(&app.worker, &format!("/v1/mindmaps/{map}")).await;
    assert_eq!(
        whole["relationships"],
        json!([]),
        "an edge to a pruned node goes with it: {whole}"
    );
}

/// An attachment is a pointer, and a node carries at most twenty.
///
/// Never the bytes: a CRDT update log is replayed by every peer that joins, so a
/// map holding a PDF would get slower to open for everybody, forever. The cap on
/// the count is the second half of that argument.
#[tokio::test]
async fn an_attachment_points_at_something_and_a_node_carries_a_bounded_number() {
    let app = TestApp::spawn().await;
    let (_, made) = app
        .post(
            &app.worker,
            "/v1/mindmaps",
            json!({ "project": "tp", "title": "Payments rebuild" }),
        )
        .await;
    let map = made["mindmap"]["id"].as_str().unwrap().to_string();
    let (_, grown) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "text": "settlement window" }),
        )
        .await;
    let node = grown["nodes"][0]["id"].as_str().unwrap().to_string();

    let (s, made) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{node}/attachments"),
            json!({
                "kind": "link",
                "name": "Scheme settlement rules",
                "gist": "T+2 for cards, same day for SEPA instant.",
                "ref": "https://example.invalid/settlement",
            }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let attachment = made["attachment"]["id"].as_str().unwrap().to_string();
    assert!(attachment.starts_with("ma-"), "{made}");
    assert_eq!(
        made["attachment"]["ref"],
        json!("https://example.invalid/settlement"),
        "{made}"
    );

    // It rides on the node, so a canvas drawing the map already has it.
    let (_, whole) = app.get(&app.worker, &format!("/v1/mindmaps/{map}")).await;
    assert_eq!(
        whole["nodes"][0]["attachments"],
        json!([{
            "id": attachment,
            "kind": "link",
            "name": "Scheme settlement rules",
            "gist": "T+2 for cards, same day for SEPA instant.",
            "ref": "https://example.invalid/settlement",
        }]),
        "{whole}"
    );

    // The kinds are an enum so a project cannot grow three spellings of "code"
    // and lose the ability to filter on it.
    let (s, refused) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{node}/attachments"),
            json!({ "kind": "spreadsheet", "name": "Q3" }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
    assert_eq!(
        refused["code"], "validation.mindmap_attachment",
        "{refused}"
    );
    assert!(
        refused["message"].as_str().unwrap().contains("table"),
        "the refusal names the kinds that would work: {refused}"
    );

    // A name is what somebody reads before deciding to follow the pointer, so an
    // attachment without one is refused rather than stored unlabelled.
    let (s, refused) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{node}/attachments"),
            json!({ "kind": "pdf", "name": "   " }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
    assert_eq!(
        refused["code"], "validation.mindmap_attachment",
        "{refused}"
    );

    let (s, gone) = app
        .delete(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{node}/attachments/{attachment}"),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{gone}");
    let (_, whole) = app.get(&app.worker, &format!("/v1/mindmaps/{map}")).await;
    assert_eq!(whole["nodes"][0]["attachments"], json!([]), "{whole}");
    let (s, gone) = app
        .delete(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{node}/attachments/{attachment}"),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{gone}");
    assert_eq!(gone["code"], "notfound.mindmap_attachment", "{gone}");

    // Twenty fit; the twenty-first is refused. A node carrying more than that
    // has become a folder.
    for n in 0..20 {
        let (s, ok) = app
            .post(
                &app.worker,
                &format!("/v1/mindmaps/{map}/nodes/{node}/attachments"),
                json!({ "kind": "code", "name": format!("fixture {n}") }),
            )
            .await;
        assert_eq!(s, StatusCode::CREATED, "attachment {n}: {ok}");
    }
    let (s, full) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{node}/attachments"),
            json!({ "kind": "code", "name": "one too many" }),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{full}");
    assert_eq!(full["code"], "mindmap.attachments_full", "{full}");
}

/// `notes` is the long form the 280-character title cap points at.
///
/// The cap is the method rather than a limitation — a branch you cannot read at
/// a glance has stopped being a brainstorm — and adding `notes` does not retire
/// that argument, it relocates it. Which is why the refusal has to name notes as
/// a way out: a cap with no escape hatch teaches people to write "…".
#[tokio::test]
async fn notes_are_the_long_form_and_the_title_refusal_now_points_at_them() {
    let app = TestApp::spawn().await;
    let (_, made) = app
        .post(
            &app.worker,
            "/v1/mindmaps",
            json!({ "project": "tp", "title": "Payments rebuild" }),
        )
        .await;
    let map = made["mindmap"]["id"].as_str().unwrap().to_string();

    let long = "Settlement is T+2 for card schemes and same-day for SEPA instant, \
                which is why the reconciliation job cannot assume one window.";
    let (s, grown) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "text": "settlement window", "notes": long }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{grown}");
    let node = grown["nodes"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(grown["nodes"][0]["notes"], json!(long), "{grown}");

    // Notes do not render in the outline — that is what keeps the title cap's
    // promise intact now that there is somewhere for detail to go.
    let (_, outline) = app
        .get(&app.worker, &format!("/v1/mindmaps/{map}/outline"))
        .await;
    let text = outline["outline"].as_str().unwrap();
    assert!(text.contains("- settlement window"), "{text}");
    assert!(
        !text.contains("SEPA"),
        "the outline stays readable at a glance: {text}"
    );

    let (s, patched) = app
        .patch(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{node}"),
            json!({ "notes": "Rewritten." }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{patched}");
    assert_eq!(patched["node"]["notes"], json!("Rewritten."), "{patched}");
    assert_eq!(
        patched["node"]["text"],
        json!("settlement window"),
        "patching notes leaves the title alone: {patched}"
    );

    // 8,000 characters is where notes stop being one thought's long form.
    let (s, refused) = app
        .patch(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{node}"),
            json!({ "notes": "n".repeat(8_001) }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
    assert_eq!(refused["code"], "validation.mindmap_notes", "{refused}");
    assert!(
        refused["message"].as_str().unwrap().contains("initiative"),
        "the refusal names where the long form belongs: {refused}"
    );
    let (s, ok) = app
        .patch(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{node}"),
            json!({ "notes": "n".repeat(8_000) }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "exactly at the cap is fine: {ok}");

    // The title is still 280, and the refusal now offers notes alongside
    // promotion rather than promotion alone.
    let (s, refused) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "text": "t".repeat(281) }),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
    assert_eq!(refused["code"], "validation.mindmap_node_text", "{refused}");
    let remedy = refused["remedy"].as_str().unwrap_or_default();
    assert!(
        remedy.contains("notes"),
        "the way out now includes the node's notes: {refused}"
    );
    assert!(
        remedy.contains("initiative"),
        "and still names promotion: {refused}"
    );
}

/// `position` is a rank among siblings on the way out and an index among them on
/// the way in.
///
/// It used to be a gapped global integer (1000, 2000, …). Gapped integers cannot
/// survive concurrency — two peers inserting between the same pair both pick
/// 1500 — so the order key underneath is now a fractional index, and the
/// published contract is the rank that integer always meant to a reader.
#[tokio::test]
async fn position_is_a_rank_among_siblings_and_places_a_node_among_them_on_write() {
    let app = TestApp::spawn().await;
    let (_, made) = app
        .post(
            &app.worker,
            "/v1/mindmaps",
            json!({ "project": "tp", "title": "Payments rebuild" }),
        )
        .await;
    let map = made["mindmap"]["id"].as_str().unwrap().to_string();

    let (_, grown) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "nodes": [{ "text": "API" }, { "text": "integrations" }] }),
        )
        .await;
    let api = grown["nodes"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(grown["nodes"][0]["position"], json!(0), "{grown}");
    assert_eq!(grown["nodes"][1]["position"], json!(1), "{grown}");

    // A second ring starts at 0 again. That is the whole difference from the old
    // column: a position is only meaningful next to its own siblings.
    let (_, children) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "nodes": [
                { "parent": api, "text": "versioning?" },
                { "parent": api, "text": "auth" },
            ] }),
        )
        .await;
    assert_eq!(children["nodes"][0]["position"], json!(0), "{children}");
    assert_eq!(children["nodes"][1]["position"], json!(1), "{children}");

    // `position` on write is an index among the new node's siblings, so 0 puts
    // it first and everything after it shifts by one.
    let (s, inserted) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "parent": api, "text": "rate limits", "position": 0 }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{inserted}");
    assert_eq!(inserted["nodes"][0]["position"], json!(0), "{inserted}");

    let ring = |whole: &Value, parent: &str| -> Vec<String> {
        let mut ring: Vec<(i64, String)> = whole["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["parent"] == json!(parent))
            .map(|n| {
                (
                    n["position"].as_i64().unwrap(),
                    n["text"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        ring.sort();
        ring.into_iter().map(|(_, text)| text).collect()
    };

    let (_, whole) = app.get(&app.worker, &format!("/v1/mindmaps/{map}")).await;
    assert_eq!(
        ring(&whole, &api),
        vec!["rate limits", "versioning?", "auth"],
        "an index of 0 puts the node first in its ring: {whole}"
    );

    // And a patch moves one within its ring without touching any other node's
    // stored order — which is the property a fractional index buys.
    let last = whole["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["text"] == json!("auth"))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (s, moved) = app
        .patch(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{last}"),
            json!({ "position": 0 }),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{moved}");
    assert_eq!(moved["node"]["position"], json!(0), "{moved}");
    let (_, whole) = app.get(&app.worker, &format!("/v1/mindmaps/{map}")).await;
    assert_eq!(
        ring(&whole, &api),
        vec!["auth", "rate limits", "versioning?"],
        "a move re-ranks the ring: {whole}"
    );
    // The first ring is untouched: the two rings are independent sequences.
    assert_eq!(
        ring(&whole, "null_parent_never_matches"),
        Vec::<String>::new()
    );
    let roots: Vec<&str> = whole["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["parent"] == Value::Null)
        .map(|n| n["text"].as_str().unwrap())
        .collect();
    assert_eq!(roots, vec!["API", "integrations"], "{whole}");
}

/// Every mindmap handler that changes the CRDT must get it on disk before it
/// answers.
///
/// A `room.mutate` writes to the document in MEMORY. The room's flusher is
/// debounced, so between the write and the flush there is a window in which the
/// room can be dropped and reloaded from its persisted updates — and everything
/// written in that window is simply gone. The handler has already answered 201.
///
/// This is not hypothetical: `add_attachment` and `promote` both shipped without
/// it. `promote` was the dangerous one, because it writes the epic to SQL and
/// only the back-link to the CRDT, so losing the back-link leaves an epic that
/// exists beside a map that has forgotten it — and the refusal that stops a
/// branch being promoted twice reads exactly that back-link, so the next call
/// would cheerfully make a second epic from the same thought.
///
/// Scanning the source is what makes this hold for the NEXT handler as well as
/// these ten. A behavioural test can only pin the routes somebody remembered to
/// write one for, which is the same memory that missed these two.
#[test]
fn every_mindmap_write_is_persisted_before_it_answers() {
    let src = include_str!("../src/api/mindmaps.rs");
    let mut missing = Vec::new();
    for part in src.split("\npub async fn ").skip(1) {
        let name = part.split('(').next().unwrap_or_default().to_string();
        // The body ends at the first column-0 closing brace.
        let body = part.split("\n}\n").next().unwrap_or_default();
        if body.contains("room.mutate") && !body.contains("persist(&state") {
            missing.push(name);
        }
    }
    assert!(
        missing.is_empty(),
        "these handlers change the document but never persist it, so the write is \
         lost if the room is dropped before the debounced flush: {missing:?}. Add \
         `persist(&state, &room, &ctx).await;` before answering."
    );
}

/// Archiving freezes the sync socket too, not just every REST route.
///
/// `can_write` is decided once, when a session ticket is minted, and a ticket
/// lives for hours. So archiving refused every REST write and refused to mint a
/// new ticket, while somebody who already had the page open went on typing into
/// a socket whose answer to "may this peer write" predated the freeze — and the
/// server applied and persisted it. Reproduced against a running server before
/// this was fixed: the project was archived, every REST path answered
/// `project.archived`, and a node title written over the socket afterwards came
/// back from `GET /v1/mindmaps/{id}`.
///
/// Asserted on CONTENT rather than on the size of the update log, because the
/// log is the wrong instrument: a refused write and a write merged into a batch
/// that was going to be flushed anyway can leave the same number of rows, and a
/// first attempt at this test passed with the fix removed for exactly that
/// reason.
///
/// Reads stay open throughout, because archiving has never restricted reading.
#[tokio::test]
async fn archiving_freezes_a_socket_that_was_already_open() {
    use docsync_support::*;
    use futures::SinkExt;
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    let app = TestApp::spawn().await;
    let (map, url) = mindmap_socket(&app, &app.admin, "Payments rebuild").await;
    let (mut peer, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("the canvas opens while the project is live");
    let mine = yrs::Doc::new();

    // One branch BEFORE the freeze, so this proves the socket was working and
    // then stopped — not that it never worked.
    let before = peer_node_update(&mine, "mn-before01", "Vor dem Archivieren", "a0");
    peer.send(WsMessage::Binary(sync_message(SYNC_UPDATE, &before).into()))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let (_, whole) = app.get(&app.worker, &format!("/v1/mindmaps/{map}")).await;
    assert!(
        mindmap_titles(&whole).contains(&"Vor dem Archivieren".to_string()),
        "the socket must work before the freeze: {whole}"
    );

    let (s, body) = app
        .post(&app.admin, "/v1/projects/tp/archive?force=true", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");

    // The socket is still open and the peer has no idea anything changed.
    let after = peer_node_update(&mine, "mn-after001", "Nach dem Archivieren", "a1");
    peer.send(WsMessage::Binary(sync_message(SYNC_UPDATE, &after).into()))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    let (s, whole) = app.get(&app.worker, &format!("/v1/mindmaps/{map}")).await;
    assert_eq!(
        s,
        StatusCode::OK,
        "reading an archived map stays allowed: {whole}"
    );
    assert!(
        !mindmap_titles(&whole).contains(&"Nach dem Archivieren".to_string()),
        "an archived project must not accept a write over a socket that was \
         already open: {whole}"
    );

    // And it thaws, because archiving is reversible by design.
    let (s, body) = app
        .post(&app.admin, "/v1/projects/tp/unarchive", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let again = peer_node_update(&mine, "mn-again001", "Wieder offen", "a2");
    peer.send(WsMessage::Binary(sync_message(SYNC_UPDATE, &again).into()))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let (_, whole) = app.get(&app.worker, &format!("/v1/mindmaps/{map}")).await;
    assert!(
        mindmap_titles(&whole).contains(&"Wieder offen".to_string()),
        "unarchiving must let the same socket write again: {whole}"
    );
}

/// Node titles as the REST read reports them.
fn mindmap_titles(whole: &Value) -> Vec<String> {
    whole["nodes"]
        .as_array()
        .map(|ns| {
            ns.iter()
                .filter_map(|n| n["text"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// A ticket minted before the freeze cannot open a writable room after it.
///
/// The sibling above keeps one socket open ACROSS the archive, which
/// `resync_frozen` covers because the room is there to be found. This is the
/// other order, and it is the one that got away: mint a ticket while the project
/// is live, archive, and only then connect. No room exists at archive time, so
/// nothing is re-asked — and the room that opens afterwards decided its own
/// writability from a comment claiming a ticket could only be minted for a
/// writable object, which is true only at the moment of minting.
///
/// Found on a running server after the first fix was already in, by re-running
/// the probe that found the original bug instead of trusting the green test.
#[tokio::test]
async fn a_ticket_minted_before_archiving_cannot_write_after_it() {
    use docsync_support::*;
    use futures::SinkExt;
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    let app = TestApp::spawn().await;
    let (map, url) = mindmap_socket(&app, &app.admin, "Payments rebuild").await;

    let (s, body) = app
        .post(&app.admin, "/v1/projects/tp/archive?force=true", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");

    // The ticket is still valid, and opening the socket still works: archiving
    // does not restrict reading, and this peer may legitimately read.
    let (mut peer, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("an archived object is still readable over its socket");
    let mine = yrs::Doc::new();
    let update = peer_node_update(&mine, "mn-stale001", "Mit altem Ticket", "a0");
    peer.send(WsMessage::Binary(sync_message(SYNC_UPDATE, &update).into()))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    let (s, whole) = app.get(&app.worker, &format!("/v1/mindmaps/{map}")).await;
    assert_eq!(s, StatusCode::OK, "{whole}");
    assert!(
        !mindmap_titles(&whole).contains(&"Mit altem Ticket".to_string()),
        "a ticket minted before the freeze must not write after it: {whole}"
    );
}

/// A flush the store refuses must not throw away what people typed.
///
/// `flush` takes the pending batch before writing it, so any failure used to end
/// with the work discarded and a single line on stderr. Nothing looked wrong at
/// the time — the room keeps serving its replica from memory, so every peer
/// still saw their own words — until the room was dropped and they were gone.
///
/// The trigger here is ordinary rather than exotic: somebody is typing when an
/// admin archives the project. Their words were written while it was live and
/// are perfectly legitimate; the flush that carries them lands after the freeze,
/// the store refuses it, and before this fix the map came back EMPTY.
///
/// Found by auditing the flush path after the archive work, not by a report.
#[tokio::test]
async fn typing_survives_a_flush_the_store_refuses() {
    use docsync_support::*;
    use futures::SinkExt;
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    let app = TestApp::spawn().await;
    let (map, url) = mindmap_socket(&app, &app.admin, "Payments rebuild").await;
    let (mut peer, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    let mine = yrs::Doc::new();

    // Legitimate typing, while the project is live.
    let u = peer_node_update(&mine, "mn-legit001", "Vor dem Archivieren", "a0");
    peer.send(WsMessage::Binary(sync_message(SYNC_UPDATE, &u).into()))
        .await
        .unwrap();
    // Let the frame actually be applied before archiving. Without this the two
    // race, and the archive can win — at which point the freeze refuses an
    // in-flight frame, which is correct behaviour and NOT what this test is
    // about. The first version had no pause and failed intermittently for
    // exactly that reason: it was exercising the freeze rather than the flush.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Archive inside the flush window, so the batch is still only in memory and
    // the write that carries it will be refused.
    let (s, body) = app
        .post(&app.admin, "/v1/projects/tp/archive?force=true", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    let (s, body) = app
        .post(&app.admin, "/v1/projects/tp/unarchive", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");

    // Empty the room, so the read below rebuilds from the log rather than from
    // the memory that would agree either way.
    drop(peer);
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    let (s, whole) = app.get(&app.worker, &format!("/v1/mindmaps/{map}")).await;
    assert_eq!(s, StatusCode::OK, "{whole}");
    assert!(
        mindmap_titles(&whole).contains(&"Vor dem Archivieren".to_string()),
        "typing done BEFORE the archive must be kept and written once the store \
         will take it again: {whole}"
    );
}

/// Revoking a token kills the sync tickets minted from it.
///
/// A `tkd_` ticket may never be more permissive than the `tk_` token it came
/// from — that is the whole basis for letting it travel in a query string. But
/// `crdt_sessions` recorded no link back to the minting token, so revocation
/// could not reach it: revoking a leaked credential left its holder writing to
/// that one object for the rest of the session's life, which is hours.
/// `revoked_at` was even read at the handshake, with a comment in the source
/// saying nothing wrote it.
///
/// Reproduced on a running server: a token was minted, took a ticket, was
/// revoked, answered 401 on REST, and its ticket then opened a socket and wrote
/// a node that `GET /v1/mindmaps/{id}` handed back.
#[tokio::test]
async fn revoking_a_token_kills_the_sync_tickets_it_minted() {
    let app = TestApp::spawn().await;
    let (map, _) = mindmap_socket(&app, &app.admin, "Payments rebuild").await;

    let (s, minted) = app
        .post(
            &app.admin,
            "/v1/tokens",
            json!({ "actor": "leaky", "scopes": ["read", "write"] }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{minted}");
    let leaky = minted["token"].as_str().unwrap().to_string();
    let leaky_id = minted["id"].as_str().unwrap().to_string();

    let (s, sess) = app
        .post(&leaky, &format!("/v1/mindmaps/{map}/session"), json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{sess}");
    let ticket = sess["token"].as_str().unwrap().to_string();
    assert_eq!(sess["can_write"], json!(true), "{sess}");

    let (s, _) = app
        .delete(&app.admin, &format!("/v1/tokens/{leaky_id}"))
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // The ticket must now be refused at the handshake, exactly as the token is
    // refused on REST.
    let ws = app.base.replacen("http://", "ws://", 1);
    let url = format!("{ws}/v1/docsync/{map}?ticket={ticket}");
    assert!(
        tokio_tungstenite::connect_async(&url).await.is_err(),
        "a ticket from a revoked token must not open a socket"
    );
}

/// Deleting a map or its project takes the plan's history with it.
///
/// `plan_trace.text` is what a section SAID at the moment somebody changed it —
/// that is the whole point of the column, and it means the history is a copy of
/// the prose. Nothing deleted from that table, so a deleted project left every
/// section's words behind in it, which is exactly what `delete_project`'s own
/// comment says must not happen: "delete the project" has to mean the text goes
/// too. The CRDT log was purged and the copy of it was not.
///
/// Reproduced on a running server before the fix: a project was created, a
/// sentence written into a node, the project deleted with `?force=true`, and the
/// sentence read straight back out of `plan_trace`.
#[tokio::test]
async fn deleting_a_project_takes_the_plan_history_with_it() {
    let app = TestApp::spawn().await;
    let (map, _) = mindmap_socket(&app, &app.admin, "Payments rebuild").await;

    let (s, made) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "text": "secret plan" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let node = made["nodes"][0]["id"].as_str().unwrap().to_string();
    let (s, _) = app
        .patch(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{node}"),
            json!({ "notes": "CONFIDENTIAL: the acquisition price is 4.2m" }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);

    let (_, trace) = app
        .get(&app.worker, &format!("/v1/mindmaps/{map}/trace"))
        .await;
    assert!(
        trace["total"].as_i64().unwrap() > 0,
        "the history must exist before we prove it is removed: {trace}"
    );

    let (s, _) = app.delete(&app.admin, "/v1/projects/tp?force=true").await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // Read straight out of SQLite, because the route that would report it is
    // gone with the project — and a row left behind is exactly the failure.
    let conn = rusqlite::Connection::open(app.db_path()).unwrap();
    let left: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM plan_trace WHERE project = 'tp'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        left, 0,
        "deleting a project must take the plan history — and its copy of the \
         prose — with it, not leave {left} row(s) behind"
    );
}

/// Compaction rewrites the log and must not touch the plan's history.
///
/// The two are different records and only one of them is the log. Compaction
/// replaces every update row with a single blob of the document's state — which
/// is why `plan_trace` exists at all: the CRDT log cannot answer "what did 2.1
/// say last Tuesday" after a compaction, and the trace can, because it keeps the
/// text on the acts that changed it.
///
/// This test exists because the author put the trace's deletion in
/// `compact_collab` instead of `purge_collab` — one function too early in the
/// same file, with the same first statement — which would have made ORDINARY
/// EDITING erase the history, silently, every 256 flushes. The whole suite
/// passed, because nothing checked that the trace survived a compaction. It does
/// now.
#[tokio::test]
async fn compaction_rewrites_the_log_and_keeps_the_history() {
    let app = TestApp::spawn().await;
    let (map, _) = mindmap_socket(&app, &app.admin, "Payments rebuild").await;

    let (s, made) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "text": "versioning" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let node = made["nodes"][0]["id"].as_str().unwrap().to_string();
    let (s, _) = app
        .patch(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes/{node}"),
            json!({ "notes": "Leaning to v1-forever with additive-only changes." }),
        )
        .await;
    assert_eq!(s, StatusCode::OK);

    let (_, before) = app
        .get(&app.worker, &format!("/v1/mindmaps/{map}/trace"))
        .await;
    let entries = before["total"].as_i64().unwrap();
    assert!(entries > 0, "history to preserve: {before}");

    // Compact directly rather than driving 256 flushes: the invariant is about
    // what compaction does, not about how many writes it takes to reach one.
    let store = app.open_store();
    let state = {
        use yrs::ReadTxn;
        let doc = yrs::Doc::new();
        let _ = doc.get_or_insert_map("nodes");
        let txn = yrs::Transact::transact(&doc);
        txn.encode_state_as_update_v1(&yrs::StateVector::default())
    };
    // Compaction refuses a log that has gained rows this caller did not write,
    // so present the honest pair: this caller loaded at seq 0 and everything
    // since is accounted to it.
    let since: i64 = rusqlite::Connection::open(app.db_path())
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM crdt_updates WHERE object_id = ?1",
            [&map],
            |r| r.get(0),
        )
        .unwrap();
    store
        .compact_collab(&map, &state, "test", 0, since)
        .expect("compact");

    let (_, after) = app
        .get(&app.worker, &format!("/v1/mindmaps/{map}/trace"))
        .await;
    assert_eq!(
        after["total"].as_i64().unwrap(),
        entries,
        "compaction rewrites the LOG; the history is a separate record and every \
         entry must survive it: {after}"
    );
}

/// A token that may only read must not change the plan by reading it.
///
/// Opening a map converts any node still carrying the legacy `notes` field into
/// a prose fragment, and `GET .../prose` used to create a fragment for a section
/// that had none. Both are WRITES — they alter the shared document, broadcast to
/// every open canvas, and land in the update log. Both ran for a `read` token.
///
/// Measured before the fix: a read-only `GET /v1/mindmaps/{id}` took the log from
/// one row to two. The browser side had made this distinction from the start
/// (`readProseOf` beside `proseOf`, with a comment saying a token that may not
/// change the plan must not change it by looking at it); the server had not.
///
/// A reader loses nothing, because the read paths fall back to the legacy field
/// and an absent fragment honestly reads as no prose.
#[tokio::test]
async fn a_read_only_token_cannot_change_the_plan_by_reading_it() {
    use docsync_support::*;
    use futures::SinkExt;
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    let app = TestApp::spawn().await;
    let (map, url) = mindmap_socket(&app, &app.admin, "Payments rebuild").await;
    let (mut peer, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    let mine = yrs::Doc::new();
    // A node written the way a canvas writes one: title and position, no prose.
    let u = peer_node_update(&mine, "mn-nopros01", "no prose here", "a0");
    peer.send(WsMessage::Binary(sync_message(SYNC_UPDATE, &u).into()))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;
    drop(peer);
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;

    let conn = rusqlite::Connection::open(app.db_path()).unwrap();
    let rows = |c: &rusqlite::Connection| -> i64 {
        c.query_row(
            "SELECT COUNT(*) FROM crdt_updates WHERE object_id = ?1",
            [&map],
            |r| r.get(0),
        )
        .unwrap()
    };
    let before = rows(&conn);
    assert!(before > 0, "the socket write must have landed first");

    let reader = app.mint("reader", &["read"], None);
    let (s, whole) = app.get(&reader, &format!("/v1/mindmaps/{map}")).await;
    assert_eq!(s, StatusCode::OK, "reading stays allowed: {whole}");
    let (s, prose) = app.get(&reader, &format!("/v1/mindmaps/{map}/prose")).await;
    assert_eq!(
        s,
        StatusCode::OK,
        "and so does reading it as a document: {prose}"
    );
    // The node is still there to be read — the point is that reading did not
    // rewrite it, not that reading returned nothing.
    assert!(
        prose["markdown"]
            .as_str()
            .unwrap()
            .contains("no prose here"),
        "the reader must still see the section: {prose}"
    );

    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    assert_eq!(
        rows(&conn),
        before,
        "a read-only token's GET must not add to the update log — reading changed \
         the shared document"
    );
}

/// A large paste is stored, and the edits after it survive.
///
/// The socket's frame cap and the store's row cap were set independently — 8 MiB
/// and 1 MiB — so a frame in between was accepted, applied and broadcast as
/// confirmed, then refused by every flush for the life of the room. With a
/// refused batch kept rather than dropped it merged with everything typed since,
/// and the map read back EMPTY.
///
/// The first fix refused the frame at the socket and closed the connection. That
/// was worse, and an independent reviewer proved it: a browser keeps its replica
/// across a reconnect and answers the handshake with the same oversized diff, so
/// the tab looped, and the ordinary edits riding in that diff were lost with the
/// paste. The test I wrote for it passed only because its reconnecting peer was a
/// FRESH `yrs::Doc`, which no browser is.
///
/// So the caps are equal now and anything the socket accepts is storable. This
/// asserts the stronger property that follows: the paste itself survives too.
#[tokio::test]
async fn a_large_paste_is_stored_and_the_edits_after_it_survive() {
    use docsync_support::*;
    use futures::SinkExt;
    use tokio_tungstenite::tungstenite::Message as WsMessage;
    use yrs::{Map, Transact};

    let app = TestApp::spawn().await;
    let (map, url) = mindmap_socket(&app, &app.admin, "Payments rebuild").await;
    let (mut peer, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    let mine = yrs::Doc::new();

    let big = {
        let nodes = mine.get_or_insert_map("nodes");
        let mut txn = mine.transact_mut();
        let entry = nodes.insert(&mut txn, "mn-bigpaste", yrs::MapPrelim::default());
        entry.insert(&mut txn, "title", "a big paste");
        entry.insert(&mut txn, "position", 0.0);
        entry.insert(&mut txn, "notes", "x".repeat(1_500_000));
        txn.encode_update_v1()
    };
    assert!(
        big.len() > 1024 * 1024,
        "the fixture must be a genuinely large paste"
    );
    assert!(
        big.len() <= takomo::store::MAX_UPDATE_BYTES,
        "and one the store can hold — the caps must agree, which is the fix"
    );
    peer.send(WsMessage::Binary(sync_message(SYNC_UPDATE, &big).into()))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // The SAME peer carries on, which is what a browser does — it does not get a
    // fresh replica between one edit and the next.
    let small = peer_node_update(&mine, "mn-ordinary1", "an ordinary thought", "a1");
    peer.send(WsMessage::Binary(sync_message(SYNC_UPDATE, &small).into()))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    drop(peer);
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    let (_, whole) = app.get(&app.worker, &format!("/v1/mindmaps/{map}")).await;
    let titles = mindmap_titles(&whole);
    assert!(
        titles.contains(&"an ordinary thought".to_string()),
        "the edit made after the paste must still be there: {whole}"
    );
    assert!(
        titles.contains(&"a big paste".to_string()),
        "and so must the paste, now that the caps agree: {whole}"
    );
}

/// A sync ticket must not outlive the token that minted it.
///
/// The TTL is 12 hours and an OAuth access token lives one, so a hosted client's
/// ticket kept WRITING to one object for eleven hours after the credential behind
/// it was dead. Revocation was handled and ordinary EXPIRY was not — and expiry
/// is the common case, since a refresh rotation or a lapsed consent leaves the
/// old token dead exactly this way.
#[tokio::test]
async fn a_sync_ticket_expires_no_later_than_its_token() {
    let app = TestApp::spawn().await;
    let (map, _) = mindmap_socket(&app, &app.admin, "Payments rebuild").await;

    let short = app.mint("agent:short", &["read", "write"], None);
    // Give that token an hour, the way an OAuth access token has one.
    let conn = rusqlite::Connection::open(app.db_path()).unwrap();
    let in_an_hour = takomo::ids::now_ms() + 3_600_000;
    conn.execute(
        "UPDATE tokens SET expires_at = ?1 WHERE actor = 'agent:short'",
        [in_an_hour],
    )
    .unwrap();

    let (s, sess) = app
        .post(&short, &format!("/v1/mindmaps/{map}/session"), json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{sess}");
    let expires: chrono::DateTime<chrono::Utc> =
        sess["expires_at"].as_str().unwrap().parse().unwrap();
    let ticket_ms = expires.timestamp_millis();
    assert!(
        ticket_ms <= in_an_hour,
        "the ticket outlives its token by {}ms",
        ticket_ms - in_an_hour
    );
}

/// A live sync session must not block the OAuth sweep.
///
/// `crdt_sessions.minted_by` references `tokens(id)`. With no ON DELETE and
/// foreign keys on, the sweep's `DELETE FROM tokens` raises a constraint failure
/// and rolls back its WHOLE transaction — so expired tokens, spent codes,
/// retired refresh rows and unused registrations all stop being reaped while one
/// such session row exists. Reachable by construction, not contrivance: token
/// retention is shorter than a session's life.
#[tokio::test]
async fn a_live_sync_session_does_not_block_the_oauth_sweep() {
    let app = TestApp::spawn().await;
    let (map, _) = mindmap_socket(&app, &app.admin, "Payments rebuild").await;

    let token = app.mint("agent:oauthish", &["read", "write"], None);
    let (s, sess) = app
        .post(&token, &format!("/v1/mindmaps/{map}/session"), json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{sess}");

    // Make it look like an expired OAuth-issued token, which is what the sweep
    // collects.
    let conn = rusqlite::Connection::open(app.db_path()).unwrap();
    let id: String = conn
        .query_row(
            "SELECT id FROM tokens WHERE actor = 'agent:oauthish'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    conn.execute(
        "INSERT INTO oauth_issued (token_id, client_id, family, refresh_hash, created_at) \
         VALUES (?1, 'client', 'fam', 'hash', 0)",
        [&id],
    )
    .unwrap();
    conn.execute("UPDATE tokens SET expires_at = 1 WHERE id = ?1", [&id])
        .unwrap();

    app.open_store()
        .sweep_expired_oauth()
        .expect("the sweep must not be blocked by a session referencing the token");

    let left: i64 = conn
        .query_row("SELECT COUNT(*) FROM tokens WHERE id = ?1", [&id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(left, 0, "the expired token should have been swept");
}

/// A refused compaction must recover, not refuse for ever.
///
/// The guard that stops compaction destroying a row this replica never replayed
/// compares where the room loaded against how much of what followed it wrote. A
/// foreign append makes those disagree — and nothing advanced the baseline, so
/// they disagreed for the life of the room: every flush refused, the log grew
/// past the compaction threshold without bound, and a full state encode plus a
/// database round-trip ran every two seconds for ever. That is the failure the
/// refusal exists to prevent, arriving by the other road.
///
/// Found by an independent reviewer, who also measured it: eight edits after one
/// foreign append took the log from 256 rows to 263, refusing every time.
#[tokio::test]
async fn a_refused_compaction_catches_up_instead_of_refusing_for_ever() {
    use docsync_support::*;
    use futures::SinkExt;
    use tokio_tungstenite::tungstenite::Message as WsMessage;
    use yrs::ReadTxn;

    let app = TestApp::spawn().await;
    let (map, url) = mindmap_socket(&app, &app.admin, "Payments rebuild").await;
    let store = app.open_store();

    let filler = {
        let doc = yrs::Doc::new();
        let _ = doc.get_or_insert_map("nodes");
        let txn = yrs::Transact::transact(&doc);
        txn.encode_state_as_update_v1(&yrs::StateVector::default())
    };
    for _ in 0..(takomo::store::COMPACT_AFTER_UPDATES - 2) {
        store.append_collab_update(&map, &filler, "pad").unwrap();
    }

    let (mut peer, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let extra = {
        use yrs::{Map, Transact};
        let doc = yrs::Doc::new();
        let nodes = doc.get_or_insert_map("nodes");
        let mut txn = doc.transact_mut();
        let entry = nodes.insert(&mut txn, "mn-fromcli1", yrs::MapPrelim::default());
        entry.insert(&mut txn, "title", "written by the CLI");
        entry.insert(&mut txn, "position", 9.0);
        txn.encode_update_v1()
    };
    store.append_collab_update(&map, &extra, "cli").unwrap();

    // Several ordinary edits, each of which would trip compaction.
    let mine = yrs::Doc::new();
    for i in 0..4 {
        let u = peer_node_update(
            &mine,
            &format!("mn-edit{i:04}"),
            &format!("edit {i}"),
            &format!("a{i}"),
        );
        peer.send(WsMessage::Binary(sync_message(SYNC_UPDATE, &u).into()))
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
    drop(peer);
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;

    let conn = rusqlite::Connection::open(app.db_path()).unwrap();
    let rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM crdt_updates WHERE object_id = ?1",
            [&map],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        rows < takomo::store::COMPACT_AFTER_UPDATES,
        "the log must have been compacted once the room caught up, not grown to {rows} rows"
    );

    // And nothing was lost getting there.
    let (_, whole) = app.get(&app.worker, &format!("/v1/mindmaps/{map}")).await;
    let titles = mindmap_titles(&whole);
    assert!(
        titles.contains(&"written by the CLI".to_string()),
        "the foreign row must survive the catch-up: {whole}"
    );
    assert!(
        titles.contains(&"edit 3".to_string()),
        "and so must the room's own last edit: {whole}"
    );
}

/// Compaction must not delete a row it never replayed.
///
/// It replaces the whole log with the room's replica, on the argument that the
/// replica holds everything the log does. True of updates this room produced,
/// false of a row appended by anything else — and `edit_mindmap_document`
/// APPENDS deliberately, so the CLI does not throw a live room's work away.
/// Compaction did the reverse damage to it.
///
/// The first guard compared row COUNTS and was a no-op: the append that precedes
/// compaction re-reads the count from the log, foreign rows included, so it
/// always agreed with itself. The test written for it passed only because it
/// called `compact_collab` directly with a deliberately stale count, which the
/// flusher never passes. An independent reviewer caught both, and the guard is on
/// `seq` identity now.
///
/// So this drives the REAL path: a room open, a foreign append, then an edit that
/// takes the log over the threshold.
#[tokio::test]
async fn compaction_does_not_drop_a_row_the_room_never_saw() {
    use docsync_support::*;
    use futures::SinkExt;
    use tokio_tungstenite::tungstenite::Message as WsMessage;
    use yrs::ReadTxn;

    let app = TestApp::spawn().await;
    let (map, url) = mindmap_socket(&app, &app.admin, "Payments rebuild").await;
    let store = app.open_store();

    // Pad the log to just under the compaction threshold, before anyone joins.
    let filler = {
        let doc = yrs::Doc::new();
        let _ = doc.get_or_insert_map("nodes");
        let txn = yrs::Transact::transact(&doc);
        txn.encode_state_as_update_v1(&yrs::StateVector::default())
    };
    for _ in 0..(takomo::store::COMPACT_AFTER_UPDATES - 2) {
        store.append_collab_update(&map, &filler, "pad").unwrap();
    }

    // Now a room opens and replays what exists.
    let (mut peer, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // Somebody else appends — the CLI's shape, out of any room.
    let extra = {
        use yrs::{Map, Transact};
        let doc = yrs::Doc::new();
        let nodes = doc.get_or_insert_map("nodes");
        let mut txn = doc.transact_mut();
        let entry = nodes.insert(&mut txn, "mn-fromcli1", yrs::MapPrelim::default());
        entry.insert(&mut txn, "title", "written by the CLI");
        entry.insert(&mut txn, "position", 9.0);
        txn.encode_update_v1()
    };
    store
        .append_collab_update(&map, &extra, "cli")
        .expect("the CLI appends");

    // One ordinary edit takes the log over the threshold, so the flusher
    // compacts — the moment the CLI's row used to disappear.
    let mine = yrs::Doc::new();
    let small = peer_node_update(&mine, "mn-ordinary1", "an ordinary thought", "a1");
    peer.send(WsMessage::Binary(sync_message(SYNC_UPDATE, &small).into()))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    drop(peer);
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    let (_, whole) = app.get(&app.worker, &format!("/v1/mindmaps/{map}")).await;
    let titles = mindmap_titles(&whole);
    assert!(
        titles.contains(&"written by the CLI".to_string()),
        "compaction destroyed a row the room never replayed: {whole}"
    );
    assert!(
        titles.contains(&"an ordinary thought".to_string()),
        "and the room's own edit must survive too: {whole}"
    );
}

/// A room opened while the archive state is moving must not settle on a stale
/// answer — in either direction.
///
/// This flag has been wrong three times, and every one of them stayed wrong for
/// the life of the room. Twice it was a stale `false` leaving an archived object
/// writable; the fix for that introduced a path that could store a stale `true`
/// and freeze a room on a LIVE project, with the socket then dropping that
/// person's typing and telling them nothing.
///
/// What it covers, stated exactly, because an earlier version of this comment
/// claimed more: it pins the PROPERTY — a room opened after an archive comes up
/// frozen, and thaws when the project does. Neutering the join's read fails it.
///
/// It does NOT cover the concurrency work those fixes were about: it passes
/// verbatim against the parent commit, so it says nothing about the ordering of
/// the decision against the install, the epoch guard, or the backstop. A
/// reviewer measured that. Those races need two commits interleaved inside a
/// microsecond window, which is reachable with fault injection and not with a
/// socket and a sleep; the honest position is that the property is pinned and
/// the mechanism is argued, not tested.
#[tokio::test]
async fn a_room_agrees_with_the_store_about_being_frozen() {
    use docsync_support::*;
    use futures::SinkExt;
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    let app = TestApp::spawn().await;
    let (map, url) = mindmap_socket(&app, &app.admin, "Payments rebuild").await;

    // Archive, then open. The room must come up frozen.
    let (s, body) = app
        .post(&app.admin, "/v1/projects/tp/archive?force=true", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let (mut peer, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("reading an archived object stays allowed");
    let mine = yrs::Doc::new();
    let u = peer_node_update(&mine, "mn-whilearch", "written while archived", "a0");
    peer.send(WsMessage::Binary(sync_message(SYNC_UPDATE, &u).into()))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let (_, whole) = app.get(&app.worker, &format!("/v1/mindmaps/{map}")).await;
    assert!(
        !mindmap_titles(&whole).contains(&"written while archived".to_string()),
        "a room opened after an archive must come up frozen: {whole}"
    );

    // Unarchive. The same room must stop being frozen — the failure mode the
    // 'never clear what you could not confirm' fallback could have made
    // permanent.
    let (s, body) = app
        .post(&app.admin, "/v1/projects/tp/unarchive", json!({}))
        .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let again = peer_node_update(&mine, "mn-afterthaw", "written after the thaw", "a1");
    peer.send(WsMessage::Binary(sync_message(SYNC_UPDATE, &again).into()))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;
    drop(peer);
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;

    let (_, whole) = app.get(&app.worker, &format!("/v1/mindmaps/{map}")).await;
    assert!(
        mindmap_titles(&whole).contains(&"written after the thaw".to_string()),
        "a room frozen by an archive must thaw with it, or somebody's typing \
         vanishes with no word to them: {whole}"
    );
}

/// Compaction must never make an object bigger, or write it past its own cap.
///
/// The escape hatch runs when an append has ALREADY been refused for size, and
/// it replaces the whole log with one row. Nothing checked the size of that row,
/// so it could take a log that was UNDER the cap and write it over — after which
/// every append is refused for every room that will ever open the object,
/// permanently and on disk. A reviewer measured it: 28,000,404 bytes in four rows
/// became 35,000,457 in one, and the escape then re-ran every two seconds.
///
/// The two guards are pinned directly rather than through a shape that produces
/// them, because "a state larger than its log" depends on Yjs encoding and a test
/// that merely happens to hit it today is not a test. A first version of this did
/// exactly that and passed for the wrong reason — the merge saved four bytes.
#[tokio::test]
async fn compaction_refuses_to_grow_an_object_or_exceed_its_cap() {
    let app = TestApp::spawn().await;
    let (map, _) = mindmap_socket(&app, &app.admin, "Payments rebuild").await;
    let store = app.open_store();

    let blob = {
        use yrs::{Map, Transact};
        let doc = yrs::Doc::new();
        let nodes = doc.get_or_insert_map("nodes");
        let mut txn = doc.transact_mut();
        let entry = nodes.insert(&mut txn, "mn-paste", yrs::MapPrelim::default());
        entry.insert(&mut txn, "title", "a paste");
        entry.insert(&mut txn, "position", 0.0);
        entry.insert(&mut txn, "notes", "x".repeat(200_000));
        txn.encode_update_v1()
    };
    store.append_collab_update(&map, &blob, "paste").unwrap();

    let conn = rusqlite::Connection::open(app.db_path()).unwrap();
    let held = || -> (i64, i64) {
        conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(bytes), 0) FROM crdt_updates WHERE object_id = ?1",
            [&map],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    };
    let before = held();

    // Frees nothing: the same bytes the log already holds.
    assert!(
        store
            .compact_collab(&map, &blob, "flusher", 0, before.0)
            .is_err(),
        "a compaction that frees nothing must be refused, or the escape retries \
         it every two seconds for ever"
    );

    // Over the cap for one object. Never decoded, so arbitrary bytes are a fair
    // stand-in for a state that large.
    //
    // Checked but NOT independently pinned, and worth saying: for any log under
    // the cap the guard above already refuses this, so removing the cap check
    // leaves this test green. It earns its place for a log that is ALREADY over
    // the cap — which a database written before these guards can hold — where it
    // is the only thing stopping the escape from writing it further over.
    let huge = vec![7u8; (takomo::store::MAX_OBJECT_BYTES + 1) as usize];
    assert!(
        store
            .compact_collab(&map, &huge, "flusher", 0, before.0)
            .is_err(),
        "compaction must not write the object past the cap it exists to get under"
    );

    assert_eq!(
        held(),
        before,
        "a refused compaction must leave the log exactly as it was"
    );

    // Emitter and consumer, bound together.
    //
    // The flusher decides whether to stop retrying by asking `is_size_refusal`,
    // and that used to be a second copy of the code string in another file — a
    // pairing the repo's drift guard cannot see, because it reads the first
    // argument of `ApiError::` constructors and an `==` is not one. A rename
    // that updated the emitters and the spec together would have left the
    // consumer stale, the guard green, and a wedged room spinning again. This is
    // what makes the two move together.
    let size_refusal = store
        .compact_collab(&map, &blob, "flusher", 0, before.0)
        .unwrap_err();
    assert!(
        takomo::store::crdt::is_size_refusal(&size_refusal),
        "a size refusal must be recognised as one, or the flusher never stops \
         retrying it: {}",
        size_refusal.body.code
    );

    // And the OTHER refusal must not be, because that one has to be retried on
    // the very next tick rather than in a minute. It needs a state that WOULD
    // free space — the size guards are checked first, so a fixture that trips
    // them never reaches the mismatch.
    let small = {
        use yrs::{Map, Transact};
        let doc = yrs::Doc::new();
        let nodes = doc.get_or_insert_map("nodes");
        let mut txn = doc.transact_mut();
        nodes.insert(&mut txn, "mn-tiny", yrs::MapPrelim::default());
        txn.encode_update_v1()
    };
    let stale = store
        .compact_collab(&map, &small, "flusher", 0, before.0 + 99)
        .unwrap_err();
    assert!(
        !takomo::store::crdt::is_size_refusal(&stale),
        "the row-mismatch refusal is not a size refusal: {}",
        stale.body.code
    );
}

/// The map and the plan must not hold different answers to "has a person looked
/// at this".
///
/// This is the property the whole branch exists to establish, and the two views
/// disagreed. The canvas draws its trust lens from the node's own `reviewed`
/// flag in the CRDT; `/documents` reads `plan_standing`, built from the trace
/// table. Nothing wrote both. Measured on a running server before the fix:
/// confirming a section from the plan left standing saying `confirmed: true`
/// while the node still said `reviewed: false`, so the map went on drawing it
/// unverified.
///
/// Found by a whole-branch review. Eleven per-commit reviews could not see it,
/// because each half was correct in the commit that added it.
#[tokio::test]
async fn confirming_a_section_agrees_across_both_views() {
    let app = TestApp::spawn().await;
    let (map, _) = mindmap_socket(&app, &app.admin, "Payments rebuild").await;

    let (s, made) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "text": "versioning" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let node = made["nodes"][0]["id"].as_str().unwrap().to_string();

    let reviewed_flag = |whole: &Value| -> bool {
        whole["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["id"] == node.as_str())
            .and_then(|n| n["reviewed"].as_bool())
            .unwrap()
    };

    let (_, before) = app.get(&app.worker, &format!("/v1/mindmaps/{map}")).await;
    assert!(!reviewed_flag(&before), "nobody has looked at it yet");

    // Confirm it the way `/documents` does.
    let (s, body) = app
        .post(
            &app.human,
            &format!("/v1/mindmaps/{map}/trace"),
            json!({ "kind": "reviewed", "node": node }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{body}");

    let (_, after) = app.get(&app.worker, &format!("/v1/mindmaps/{map}")).await;
    assert_eq!(
        after["standing"][&node]["confirmed"],
        json!(true),
        "the plan must call it confirmed: {after}"
    );
    assert!(
        reviewed_flag(&after),
        "and the MAP must agree — it drew the section unverified while the plan \
         called it confirmed: {after}"
    );
}

/// The plan's prompt bar route answers, and says so when no model is configured.
///
/// `POST /v1/mindmaps/{id}/run` was added by this branch and shipped without an
/// integration test, which this repo's own rule forbids — a whole-branch review
/// found it, eleven per-commit reviews did not. It is the one route here that
/// calls a language model, so the interesting behaviour without a key is that it
/// refuses in a way a caller can act on rather than 500ing or pretending.
///
/// The test suite configures no key, so this is the path every reader of this
/// branch will hit; the model path itself needs a live provider and is not
/// exercised here, which is worth stating rather than implying coverage.
#[tokio::test]
async fn the_plan_prompt_bar_refuses_clearly_when_no_model_is_configured() {
    let app = TestApp::spawn().await;
    let (map, _) = mindmap_socket(&app, &app.admin, "Payments rebuild").await;
    let (s, made) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/nodes"),
            json!({ "text": "versioning" }),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{made}");
    let node = made["nodes"][0]["id"].as_str().unwrap().to_string();

    let (s, body) = app
        .post(
            &app.worker,
            &format!("/v1/mindmaps/{map}/run"),
            json!({ "node": node, "instruction": "Say what additive-only forbids." }),
        )
        .await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["code"], "document.agent_not_configured", "{body}");
    assert!(
        body["remedy"].as_str().unwrap_or_default().len() > 10,
        "a refusal a caller cannot act on is not a teaching error: {body}"
    );

    // A reader may not drive it at all.
    let reader = app.mint("agent:reader", &["read"], None);
    let (s, denied) = app
        .post(
            &reader,
            &format!("/v1/mindmaps/{map}/run"),
            json!({ "node": node, "instruction": "anything" }),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{denied}");
}
