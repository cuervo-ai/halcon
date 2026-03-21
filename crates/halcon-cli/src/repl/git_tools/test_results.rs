//! Test Result Parsers — parse output from multiple test frameworks into a
//! unified [`TestSuiteResult`] representation.
//!
//! # Supported Formats
//!
//! | Format            | Entry point                     |
//! |-------------------|---------------------------------|
//! | cargo test stdout | [`parse_cargo_test`]            |
//! | JUnit XML         | [`parse_junit_xml`]             |
//! | Jest JSON (--json)| [`parse_jest_json`]             |
//!
//! All parsers are synchronous and allocation-minimal.  Async callers should
//! wrap them in `tokio::task::spawn_blocking` when processing large files.

use serde::{Deserialize, Serialize};

// ── TestStatus ────────────────────────────────────────────────────────────────

/// Pass/fail/skip disposition of a single test case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TestStatus {
    Passed,
    Failed,
    Ignored,
    /// Test was reported as pending / todo by the framework.
    Pending,
}

impl TestStatus {
    /// True when the test represents a failure that should block integration.
    pub fn is_blocking_failure(&self) -> bool {
        matches!(self, TestStatus::Failed)
    }
}

// ── TestCase ──────────────────────────────────────────────────────────────────

/// A single test case result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestCase {
    /// Fully-qualified test name (e.g. `module::tests::my_test`).
    pub name: String,
    /// Test outcome.
    pub status: TestStatus,
    /// Duration in milliseconds, if reported by the framework.
    pub duration_ms: Option<f64>,
    /// Failure message / stdout captured for failing tests.
    pub failure_message: Option<String>,
}

// ── TestSuiteResult ───────────────────────────────────────────────────────────

/// Aggregated result for one test suite run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestSuiteResult {
    /// Human-readable suite name (crate name, test file, etc.).
    pub suite_name: String,
    /// Individual test outcomes.
    pub cases: Vec<TestCase>,
    /// True when *no* cases have `TestStatus::Failed`.
    pub all_passed: bool,
    /// Total wall-clock duration in milliseconds, if reported.
    pub total_duration_ms: Option<f64>,
    /// Source format this was parsed from.
    pub format: TestResultFormat,
}

impl TestSuiteResult {
    /// Number of passed tests.
    pub fn passed(&self) -> usize {
        self.cases
            .iter()
            .filter(|c| c.status == TestStatus::Passed)
            .count()
    }

    /// Number of failed tests.
    pub fn failed(&self) -> usize {
        self.cases
            .iter()
            .filter(|c| c.status == TestStatus::Failed)
            .count()
    }

    /// Number of ignored / skipped tests.
    pub fn ignored(&self) -> usize {
        self.cases
            .iter()
            .filter(|c| matches!(c.status, TestStatus::Ignored | TestStatus::Pending))
            .count()
    }

    /// One-line summary string.
    pub fn summary(&self) -> String {
        let label = if self.all_passed { "PASS" } else { "FAIL" };
        format!(
            "[{}] {} — {} passed, {} failed, {} ignored",
            label,
            self.suite_name,
            self.passed(),
            self.failed(),
            self.ignored(),
        )
    }
}

// ── TestResultFormat ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TestResultFormat {
    CargoTest,
    JunitXml,
    JestJson,
}

// ── cargo test parser ─────────────────────────────────────────────────────────

/// Parse the plain-text output from `cargo test`.
///
/// Recognises lines of the form:
/// ```text
/// test module::name ... ok
/// test module::name ... FAILED
/// test module::name ... ignored
/// ```
/// as well as the final summary line:
/// ```text
/// test result: ok. 42 passed; 3 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.12s
/// ```
pub fn parse_cargo_test(output: &str, suite_name: &str) -> TestSuiteResult {
    let mut cases = Vec::new();
    let mut total_duration_ms: Option<f64> = None;

    for line in output.lines() {
        let line = line.trim();

        // Individual test line: "test foo::bar ... ok"
        if let Some(rest) = line.strip_prefix("test ") {
            if let Some(name_and_result) = parse_cargo_test_line(rest) {
                cases.push(name_and_result);
                continue;
            }
        }

        // Summary line: "test result: ok. N passed; ..."
        if line.starts_with("test result:") {
            total_duration_ms = parse_cargo_summary_duration(line);
        }
    }

    let all_passed = cases.iter().all(|c| c.status != TestStatus::Failed);
    TestSuiteResult {
        suite_name: suite_name.to_string(),
        cases,
        all_passed,
        total_duration_ms,
        format: TestResultFormat::CargoTest,
    }
}

fn parse_cargo_test_line(rest: &str) -> Option<TestCase> {
    // rest = "module::name ... ok" or "module::name ... FAILED"
    let dot_pos = rest.rfind(" ... ")?;
    let name = rest[..dot_pos].trim().to_string();
    let outcome = rest[dot_pos + 5..].trim();

    let status = match outcome {
        "ok" => TestStatus::Passed,
        "FAILED" => TestStatus::Failed,
        "ignored" => TestStatus::Ignored,
        _ => return None,
    };

    Some(TestCase {
        name,
        status,
        duration_ms: None,
        failure_message: None,
    })
}

fn parse_cargo_summary_duration(line: &str) -> Option<f64> {
    // "... finished in 0.12s"
    let prefix = "finished in ";
    let pos = line.rfind(prefix)?;
    let tail = &line[pos + prefix.len()..];
    let seconds_str = tail.trim_end_matches('s').trim();
    seconds_str.parse::<f64>().ok().map(|s| s * 1000.0)
}

// ── JUnit XML parser ──────────────────────────────────────────────────────────

/// Parse a JUnit XML report (`<testsuite>` root or `<testsuites>` wrapper).
///
/// Only the outermost `<testsuite>` is processed if multiple are present; for
/// multi-suite files use multiple calls.
pub fn parse_junit_xml(xml: &str, suite_name_fallback: &str) -> TestSuiteResult {
    let suite_name = extract_xml_attr(xml, "testsuite", "name")
        .unwrap_or_else(|| suite_name_fallback.to_string());

    let total_duration_ms = extract_xml_attr(xml, "testsuite", "time")
        .and_then(|t| t.parse::<f64>().ok())
        .map(|s| s * 1000.0);

    let mut cases = Vec::new();
    let mut cursor = xml;

    while let Some(tc_start) = cursor.find("<testcase") {
        cursor = &cursor[tc_start..];
        let tc_end = cursor.find('>').unwrap_or(cursor.len());
        let tag_head = &cursor[..=tc_end];

        let classname = extract_inline_attr(tag_head, "classname").unwrap_or_default();
        let test_name = extract_inline_attr(tag_head, "name").unwrap_or_default();
        let duration_ms = extract_inline_attr(tag_head, "time")
            .and_then(|t| t.parse::<f64>().ok())
            .map(|s| s * 1000.0);

        let name = if classname.is_empty() {
            test_name
        } else {
            format!("{classname}::{test_name}")
        };

        // Determine whether this is a self-closing tag (<testcase ... />).
        let is_self_closing = tag_head.ends_with("/>");

        let (status, failure_message, advance) = if is_self_closing {
            // Self-closing: no body, no </testcase>. Advance past the tag.
            (TestStatus::Passed, None, tc_end + 1)
        } else {
            // Has a body — find </testcase> from the current cursor position.
            let body_start = tc_end + 1;
            let close_pos = cursor.find("</testcase>");
            let body_end = close_pos.unwrap_or(cursor.len());
            let body = if body_start < body_end {
                &cursor[body_start..body_end]
            } else {
                ""
            };

            let outcome = if body.contains("<failure") || body.contains("<error") {
                let msg = extract_element_text(body, "failure")
                    .or_else(|| extract_element_text(body, "error"));
                (TestStatus::Failed, msg)
            } else if body.contains("<skipped") {
                (TestStatus::Ignored, None)
            } else {
                (TestStatus::Passed, None)
            };

            // Advance past </testcase>.
            let adv = close_pos.map(|p| p + 11).unwrap_or(cursor.len());
            (outcome.0, outcome.1, adv)
        };

        cases.push(TestCase {
            name,
            status,
            duration_ms,
            failure_message,
        });

        cursor = &cursor[advance.max(1)..];
    }

    let all_passed = cases.iter().all(|c| c.status != TestStatus::Failed);
    TestSuiteResult {
        suite_name,
        cases,
        all_passed,
        total_duration_ms,
        format: TestResultFormat::JunitXml,
    }
}

// Minimal XML helpers — avoids pulling in a full XML parser crate.

fn extract_xml_attr(xml: &str, element: &str, attr: &str) -> Option<String> {
    let open = format!("<{element}");
    let pos = xml.find(&open)?;
    let tag_end = xml[pos..].find('>')?;
    let tag = &xml[pos..pos + tag_end];
    extract_inline_attr(tag, attr)
}

fn extract_inline_attr(tag: &str, attr: &str) -> Option<String> {
    // Use " attr=\"" (space-prefixed) to avoid matching attr as a suffix of another attribute
    // name (e.g. searching "name" would otherwise match inside "classname=").
    // There is always a space before any attribute in a well-formed XML/HTML tag.
    let key = format!(" {attr}=\"");
    let start = tag.find(&key)? + key.len();
    let end = tag[start..].find('"')?;
    Some(tag[start..start + end].to_string())
}

fn extract_element_text(xml: &str, element: &str) -> Option<String> {
    let open = format!("<{element}");
    let close = format!("</{element}>");
    let start_tag_pos = xml.find(&open)?;
    let tag_close = xml[start_tag_pos..].find('>')?;
    let body_start = start_tag_pos + tag_close + 1;
    let body_end = xml.find(&close)?;
    if body_end > body_start {
        Some(xml[body_start..body_end].trim().to_string())
    } else {
        None
    }
}

// ── Jest JSON parser ──────────────────────────────────────────────────────────

/// Parse Jest's `--json` output format.
///
/// Jest JSON root has the shape:
/// ```json
/// {
///   "numPassedTests": 10,
///   "numFailedTests": 2,
///   "testResults": [
///     {
///       "testFilePath": "...",
///       "testResults": [
///         { "fullName": "...", "status": "passed", "duration": 12 }
///       ]
///     }
///   ]
/// }
/// ```
pub fn parse_jest_json(json: &str, suite_name: &str) -> TestSuiteResult {
    // Minimal hand-rolled parser — avoids serde dependency on the caller side.
    // Delegates to serde_json for robustness.
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(root) => parse_jest_value(&root, suite_name),
        Err(_) => TestSuiteResult {
            suite_name: suite_name.to_string(),
            cases: vec![],
            all_passed: false,
            total_duration_ms: None,
            format: TestResultFormat::JestJson,
        },
    }
}

fn parse_jest_value(root: &serde_json::Value, suite_name: &str) -> TestSuiteResult {
    let mut cases = Vec::new();

    let test_suites = root
        .get("testResults")
        .and_then(|v| v.as_array())
        .map(|a| a.as_slice())
        .unwrap_or(&[]);

    for suite in test_suites {
        let tests = suite
            .get("testResults")
            .or_else(|| suite.get("assertionResults"))
            .and_then(|v| v.as_array())
            .map(|a| a.as_slice())
            .unwrap_or(&[]);

        for test in tests {
            let name = test
                .get("fullName")
                .or_else(|| test.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or("<unknown>")
                .to_string();

            let raw_status = test
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            let status = match raw_status {
                "passed" => TestStatus::Passed,
                "failed" => TestStatus::Failed,
                "pending" | "todo" | "skipped" => TestStatus::Pending,
                _ => TestStatus::Ignored,
            };

            let duration_ms = test.get("duration").and_then(|v| v.as_f64());

            let failure_message = if status == TestStatus::Failed {
                test.get("failureMessages")
                    .and_then(|v| v.as_array())
                    .and_then(|msgs| msgs.first())
                    .and_then(|m| m.as_str())
                    .map(|s| s.to_string())
            } else {
                None
            };

            cases.push(TestCase {
                name,
                status,
                duration_ms,
                failure_message,
            });
        }
    }

    let all_passed = cases.iter().all(|c| c.status != TestStatus::Failed);

    // Jest reports total duration in the `testExecError` / `perfStats` block.
    let total_duration_ms = root.get("startTime").and(None::<f64>); // Accurate duration requires end time; skip for now.

    TestSuiteResult {
        suite_name: suite_name.to_string(),
        cases,
        all_passed,
        total_duration_ms,
        format: TestResultFormat::JestJson,
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── cargo test ─────────────────────────────────────────────────────────────

    const CARGO_OUTPUT_ALL_PASS: &str = r#"
running 3 tests
test module::tests::alpha ... ok
test module::tests::beta ... ok
test module::tests::gamma ... ignored

test result: ok. 2 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.05s
"#;

    const CARGO_OUTPUT_WITH_FAILURE: &str = r#"
running 4 tests
test module::alpha ... ok
test module::beta ... FAILED
test module::gamma ... ok
test module::delta ... ignored

failures:

---- module::beta stdout ----
thread 'module::beta' panicked at 'assertion failed', src/lib.rs:42

test result: FAILED. 2 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.12s
"#;

    #[test]
    fn cargo_all_pass() {
        let r = parse_cargo_test(CARGO_OUTPUT_ALL_PASS, "my_crate");
        assert!(r.all_passed);
        assert_eq!(r.passed(), 2);
        assert_eq!(r.failed(), 0);
        assert_eq!(r.ignored(), 1);
        assert_eq!(r.cases.len(), 3);
        assert_eq!(r.format, TestResultFormat::CargoTest);
    }

    #[test]
    fn cargo_with_failure() {
        let r = parse_cargo_test(CARGO_OUTPUT_WITH_FAILURE, "my_crate");
        assert!(!r.all_passed);
        assert_eq!(r.passed(), 2);
        assert_eq!(r.failed(), 1);
        assert_eq!(r.ignored(), 1);
        let failed = r
            .cases
            .iter()
            .find(|c| c.status == TestStatus::Failed)
            .unwrap();
        assert_eq!(failed.name, "module::beta");
    }

    #[test]
    fn cargo_summary_duration_parsed() {
        let r = parse_cargo_test(CARGO_OUTPUT_ALL_PASS, "x");
        assert!(r.total_duration_ms.is_some());
        let ms = r.total_duration_ms.unwrap();
        assert!((ms - 50.0).abs() < 1.0, "expected ~50ms, got {ms}");
    }

    #[test]
    fn cargo_empty_output() {
        let r = parse_cargo_test("", "empty");
        assert!(r.all_passed, "empty output → no failures");
        assert_eq!(r.cases.len(), 0);
    }

    #[test]
    fn cargo_summary_line() {
        let r = parse_cargo_test(CARGO_OUTPUT_WITH_FAILURE, "suite");
        // Duration: 0.12s → 120ms
        let ms = r.total_duration_ms.unwrap();
        assert!((ms - 120.0).abs() < 1.0, "expected 120ms, got {ms}");
    }

    // ── JUnit XML ──────────────────────────────────────────────────────────────

    const JUNIT_ALL_PASS: &str = r#"<?xml version="1.0"?>
<testsuite name="MyTestSuite" tests="2" time="0.234">
  <testcase classname="com.example.Foo" name="testAlpha" time="0.1"/>
  <testcase classname="com.example.Foo" name="testBeta" time="0.134"/>
</testsuite>"#;

    const JUNIT_WITH_FAILURE: &str = r#"<?xml version="1.0"?>
<testsuite name="FailSuite" tests="2" time="0.5">
  <testcase classname="pkg.Bar" name="testOk" time="0.1"/>
  <testcase classname="pkg.Bar" name="testFail" time="0.4">
    <failure message="assertion failed">Expected 1 but was 2</failure>
  </testcase>
</testsuite>"#;

    const JUNIT_SKIPPED: &str = r#"<?xml version="1.0"?>
<testsuite name="SkipSuite" tests="1" time="0.01">
  <testcase classname="pkg.Skip" name="testSkipped" time="0.0">
    <skipped/>
  </testcase>
</testsuite>"#;

    #[test]
    fn junit_all_pass() {
        let r = parse_junit_xml(JUNIT_ALL_PASS, "fallback");
        assert!(r.all_passed);
        assert_eq!(r.suite_name, "MyTestSuite");
        assert_eq!(r.cases.len(), 2);
        assert_eq!(r.passed(), 2);
        assert_eq!(r.format, TestResultFormat::JunitXml);
    }

    #[test]
    fn junit_duration_parsed() {
        let r = parse_junit_xml(JUNIT_ALL_PASS, "x");
        let ms = r.total_duration_ms.unwrap();
        assert!((ms - 234.0).abs() < 1.0, "expected 234ms, got {ms}");
    }

    #[test]
    fn junit_with_failure() {
        let r = parse_junit_xml(JUNIT_WITH_FAILURE, "fallback");
        assert!(!r.all_passed);
        assert_eq!(r.failed(), 1);
        let failed = r
            .cases
            .iter()
            .find(|c| c.status == TestStatus::Failed)
            .unwrap();
        assert_eq!(failed.name, "pkg.Bar::testFail");
        assert!(failed
            .failure_message
            .as_deref()
            .unwrap_or("")
            .contains("Expected 1"));
    }

    #[test]
    fn junit_skipped() {
        let r = parse_junit_xml(JUNIT_SKIPPED, "fallback");
        assert!(r.all_passed);
        assert_eq!(r.ignored(), 1);
    }

    #[test]
    fn junit_fallback_name() {
        let no_name = r#"<testsuite tests="0" time="0.0"></testsuite>"#;
        let r = parse_junit_xml(no_name, "fallback_name");
        assert_eq!(r.suite_name, "fallback_name");
    }

    // ── Jest JSON ──────────────────────────────────────────────────────────────

    const JEST_ALL_PASS: &str = r#"
{
  "numPassedTests": 2,
  "numFailedTests": 0,
  "testResults": [
    {
      "testFilePath": "/repo/src/foo.test.js",
      "testResults": [
        {"fullName": "Foo renders correctly", "status": "passed", "duration": 15},
        {"fullName": "Foo handles errors", "status": "passed", "duration": 7}
      ]
    }
  ]
}"#;

    const JEST_WITH_FAILURE: &str = r#"
{
  "numPassedTests": 1,
  "numFailedTests": 1,
  "testResults": [
    {
      "testFilePath": "/repo/src/bar.test.js",
      "testResults": [
        {"fullName": "Bar passes", "status": "passed", "duration": 5},
        {"fullName": "Bar fails",  "status": "failed",  "duration": 3,
         "failureMessages": ["Expected true to be false"]}
      ]
    }
  ]
}"#;

    const JEST_PENDING: &str = r#"
{
  "testResults": [
    {
      "testResults": [
        {"fullName": "todo test", "status": "todo", "duration": 0}
      ]
    }
  ]
}"#;

    #[test]
    fn jest_all_pass() {
        let r = parse_jest_json(JEST_ALL_PASS, "frontend");
        assert!(r.all_passed);
        assert_eq!(r.passed(), 2);
        assert_eq!(r.failed(), 0);
        assert_eq!(r.format, TestResultFormat::JestJson);
    }

    #[test]
    fn jest_with_failure() {
        let r = parse_jest_json(JEST_WITH_FAILURE, "frontend");
        assert!(!r.all_passed);
        assert_eq!(r.failed(), 1);
        let f = r
            .cases
            .iter()
            .find(|c| c.status == TestStatus::Failed)
            .unwrap();
        assert_eq!(f.name, "Bar fails");
        assert!(f
            .failure_message
            .as_deref()
            .unwrap_or("")
            .contains("Expected true"));
    }

    #[test]
    fn jest_duration_per_test() {
        let r = parse_jest_json(JEST_ALL_PASS, "frontend");
        let first = &r.cases[0];
        assert_eq!(first.duration_ms, Some(15.0));
    }

    #[test]
    fn jest_pending_status() {
        let r = parse_jest_json(JEST_PENDING, "frontend");
        assert!(r.all_passed, "pending tests should not block");
        assert_eq!(r.cases[0].status, TestStatus::Pending);
        assert_eq!(r.ignored(), 1);
    }

    #[test]
    fn jest_malformed_json() {
        let r = parse_jest_json("{{{invalid", "frontend");
        assert!(!r.all_passed, "malformed input treated as failure");
        assert_eq!(r.cases.len(), 0);
    }

    // ── shared ─────────────────────────────────────────────────────────────────

    #[test]
    fn test_status_blocking_failure_only_failed() {
        assert!(TestStatus::Failed.is_blocking_failure());
        assert!(!TestStatus::Passed.is_blocking_failure());
        assert!(!TestStatus::Ignored.is_blocking_failure());
        assert!(!TestStatus::Pending.is_blocking_failure());
    }

    #[test]
    fn suite_summary_pass() {
        let r = parse_cargo_test(CARGO_OUTPUT_ALL_PASS, "my_crate");
        let s = r.summary();
        assert!(s.contains("[PASS]"));
        assert!(s.contains("my_crate"));
        assert!(s.contains("2 passed"));
    }

    #[test]
    fn suite_summary_fail() {
        let r = parse_cargo_test(CARGO_OUTPUT_WITH_FAILURE, "my_crate");
        let s = r.summary();
        assert!(s.contains("[FAIL]"));
        assert!(s.contains("1 failed"));
    }
}
