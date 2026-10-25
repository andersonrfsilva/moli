use anyhow::Result;
use axum::{
    Router,
    http::HeaderMap,
    response::{Html, IntoResponse, Redirect},
    routing::get,
};
use clap::Parser;
use moli::{app, cli::Cli};
use std::{
    io::{self, Write},
    process::{Command, Output, Stdio},
};
use tokio::{net::TcpListener, task::JoinHandle};

struct CookieFixture {
    base: String,
    task: JoinHandle<()>,
}

impl CookieFixture {
    async fn spawn() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let app = Router::new()
            .route(
                "/echo",
                get(|headers: HeaderMap| async move {
                    let cookie = headers
                        .get("cookie")
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or("");
                    (
                        [("set-cookie", "session=fixture; Path=/; HttpOnly")],
                        Html(format!("<main>{cookie}</main>")),
                    )
                }),
            )
            .route(
                "/raw",
                get(|| async {
                    (
                        [
                            ("content-type", "application/octet-stream"),
                            ("set-cookie", "session=fixture; Path=/"),
                        ],
                        b"raw body".as_slice(),
                    )
                }),
            )
            .route("/redirect", get(|| async { Redirect::temporary("/echo") }))
            .route(
                "/cross-host",
                get(move || async move {
                    Redirect::temporary(&format!("http://localhost:{port}/echo"))
                }),
            )
            .route(
                "/cache",
                get(|headers: HeaderMap| async move {
                    let cookie = headers
                        .get("cookie")
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or("");
                    (
                        [("cache-control", "public, max-age=3600")],
                        Html(format!("<main>{cookie}</main>")),
                    )
                        .into_response()
                }),
            );
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Ok(Self {
            base: format!("http://127.0.0.1:{port}"),
            task,
        })
    }

    fn run(&self, path: &str, args: &[&str], stdin: &[u8]) -> Result<Output> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_moli"))
            .args([
                "fetch",
                "--http-no-proxy",
                "*",
                "--redirect-wait-ms",
                "0",
                "--wait-until",
                "load",
                "--timeout",
                "5000",
            ])
            .args(args)
            .arg(format!("{}{path}", self.base))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child.stdin.take().unwrap().write_all(stdin)?;
        Ok(child.wait_with_output()?)
    }
}

impl Drop for CookieFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn record() -> &'static str {
    "127.0.0.1\tFALSE\t/\tFALSE\t0\tfile\tloaded\n"
}

fn assert_success(output: &Output) -> String {
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn cookie_short_options_are_rejected_without_overwriting_existing_files() -> Result<()> {
    let path = super::unique_temp_file_path("cookie-old-short", "cookies.txt")?;
    std::fs::write(&path, record())?;
    for flag in ["-b", "-c"] {
        let error = Cli::try_parse_from([
            "moli",
            "fetch",
            flag,
            &path.to_string_lossy(),
            "https://example.test/",
        ])
        .unwrap_err();
        assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
        assert_eq!(std::fs::read_to_string(&path)?, record());
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cookie_inputs_support_literals_files_stdin_and_missing_files() -> Result<()> {
    let fixture = CookieFixture::spawn().await?;
    let file = super::unique_temp_file_path("curl-cookie-input", "cookies.txt")?;
    std::fs::write(&file, format!("invalid record\n{}", record()))?;
    let file = file.to_string_lossy();
    let missing = super::unique_temp_file_path("missing-cookie-input", "missing.txt")?;
    let directory = missing.parent().unwrap().to_string_lossy();
    for flag in ["--cookie", "--cookie-file"] {
        let cases = [
            (vec![flag, "a=1; b=2"], "a=1; b=2"),
            (vec![flag, "a=1", flag, "b=2"], "a=1;b=2"),
            (vec![flag, &file, flag, "a=1"], "file=loaded; a=1"),
            (vec![flag, "-"], "file=loaded"),
            (vec![flag, ""], ""),
        ];
        for (args, expected) in cases {
            let body = assert_success(&fixture.run("/echo", &args, record().as_bytes())?);
            assert!(body.contains(&format!("<main>{expected}</main>")), "{body}");
        }
        for source in [missing.to_string_lossy().as_ref(), directory.as_ref()] {
            let body = assert_success(&fixture.run("/echo", &[flag, source], b"")?);
            assert!(body.contains("<main></main>"), "{body}");
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn literal_cookies_follow_redirects_without_entering_the_exported_store() -> Result<()> {
    let fixture = CookieFixture::spawn().await?;
    for path in ["/redirect", "/cross-host"] {
        let output = fixture.run(
            path,
            &["--cookie", "literal=sent", "--cookie-jar", "-"],
            b"",
        )?;
        let body = assert_success(&output);
        let (response, jar) = body.split_once("# Netscape HTTP Cookie File").unwrap();
        assert!(response.contains("<main>literal=sent</main>"), "{body}");
        assert!(jar.contains("\tsession\tfixture\n"), "{body}");
        assert!(!jar.contains("\tliteral\t"), "{body}");
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn literal_cookies_do_not_reuse_another_requests_http_cache() -> Result<()> {
    let fixture = CookieFixture::spawn().await?;
    let cache = super::unique_temp_dir("literal-cookie-cache")?;
    for cookie in ["a=1", "a=2"] {
        let body = assert_success(&fixture.run(
            "/cache",
            &[
                "--cookie",
                cookie,
                "--http-cache-dir",
                &cache.to_string_lossy(),
            ],
            b"",
        )?);
        assert!(body.contains(&format!("<main>{cookie}</main>")), "{body}");
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cookie_export_runs_after_evaluation_and_raw_output_errors() -> Result<()> {
    let fixture = CookieFixture::spawn().await?;
    for (path, args) in [
        (
            "/echo",
            vec![
                "--eval",
                "(() => { throw new Error('cookie-evaluation-failure'); })()",
            ],
        ),
        ("/raw", vec!["--dump", "html"]),
    ] {
        let jar = super::unique_temp_file_path("cookie-error-export", "cookies.txt")?;
        let jar_arg = jar.to_string_lossy();
        let mut args = args;
        args.extend(["--cookie-jar", &jar_arg]);
        let output = fixture.run(path, &args, b"")?;
        assert!(!output.status.success(), "{output:?}");
        assert!(std::fs::read_to_string(jar)?.contains("\tsession\tfixture\n"));
    }
    Ok(())
}

struct FailingOutput {
    flush: bool,
}
impl Write for FailingOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.flush {
            Ok(bytes.len())
        } else {
            Err(io::Error::other("cookie test write failure"))
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::other("cookie test flush failure"))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cookie_export_runs_after_response_write_and_flush_errors() -> Result<()> {
    let fixture = CookieFixture::spawn().await?;
    for path in ["/echo", "/raw"] {
        for flush in [false, true] {
            let jar = super::unique_temp_file_path("cookie-io-error", "cookies.txt")?;
            let url = format!("{}{path}", fixture.base);
            let cli = Cli::try_parse_from([
                "moli",
                "fetch",
                "--http-no-proxy",
                "*",
                "--redirect-wait-ms",
                "0",
                "--cookie-jar",
                &jar.to_string_lossy(),
                &url,
            ])?;
            let error = app::run_cli(&mut FailingOutput { flush }, cli)
                .await
                .unwrap_err();
            assert!(error.downcast_ref::<io::Error>().is_some(), "{error:?}");
            assert!(std::fs::read_to_string(jar)?.contains("\tsession\tfixture\n"));
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cookie_export_failure_does_not_change_the_transfer_result() -> Result<()> {
    let fixture = CookieFixture::spawn().await?;
    let jar = super::unique_temp_file_path("cookie-unwritable", "missing/parent/cookies.txt")?;
    assert_success(&fixture.run("/echo", &["--cookie-jar", &jar.to_string_lossy()], b"")?);
    assert!(!jar.exists());
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cookie_export_uses_the_last_destination_and_replaces_empty_jars() -> Result<()> {
    let fixture = CookieFixture::spawn().await?;
    let first = super::unique_temp_file_path("first-cookie-jar", "cookies.txt")?;
    let last = super::unique_temp_file_path("last-cookie-jar", "cookies.txt")?;
    std::fs::write(&first, "preserve this file")?;
    std::fs::write(&last, record())?;
    let output = fixture.run(
        "/cache",
        &[
            "--cookie-jar",
            &first.to_string_lossy(),
            "--cookie-jar",
            &last.to_string_lossy(),
        ],
        b"",
    )?;
    assert_success(&output);
    assert_eq!(std::fs::read_to_string(first)?, "preserve this file");
    assert!(moli_cookie_cache::load_cookie_file(last)?.is_empty());
    Ok(())
}

#[test]
fn cookie_stdin_is_not_shared_with_script_stdin() -> Result<()> {
    for option in ["--eval-file", "--wait-script-file"] {
        let output = Command::new(env!("CARGO_BIN_EXE_moli"))
            .args([
                "fetch",
                "--cookie",
                "-",
                option,
                "-",
                "https://cookie-stdin-conflict.invalid/",
            ])
            .stdin(Stdio::null())
            .output()?;
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("cannot share stdin"),
            "{output:?}"
        );
        assert!(output.stdout.is_empty());
    }
    Ok(())
}
