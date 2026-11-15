use super::*;

#[test]
fn redirected_auto_and_explicit_toml_preserve_the_complete_payload() -> io::Result<()> {
    let body = "requests = 12\nfuture_counter = 9\n[process]\npid = 42\n";
    for (terminal, format) in [
        (false, Format::Auto),
        (false, Format::Toml),
        (true, Format::Toml),
    ] {
        let output = Output {
            terminal,
            color: true,
        };
        assert_eq!(
            output.render(&Command::Stats, format, body, Path::new("test.sock"))?,
            body
        );
    }
    Ok(())
}

#[test]
fn stats_distinguish_process_memory_cache_weight_and_historical_failures() -> io::Result<()> {
    let output = Output {
        terminal: true,
        color: false,
    };
    let body = "uptime_secs = 90061\nrequests = 1000\nblocked = 100\ncache_hits = 25\nerrors = 2\nupstream_failures = 1\nfilter_rules = 193201\ncache_estimated_bytes = 141312\n[process]\npid = 42\nversion = 'test'\nos = 'linux'\narch = 'aarch64'\nrss_bytes = 39976960\npeak_rss_bytes = 52428800\ndata_bytes = 62914560\nthreads = 5\n";
    let text = output.render(&Command::Stats, Format::Auto, body, Path::new("test.sock"))?;
    for expected in [
        "RSS actuelle",
        "38.12 Mio",
        "Poids estimé du cache",
        "138.00 Kio",
        "193 201",
        "10.0 %",
        "2.5 %",
        "1 j 01 h 01 min 01 s",
        "ne prouvent pas une panne actuelle",
        "reloads exclut",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    assert!(!text.contains('\x1b'));
    assert!(!text.contains("NaN"));
    Ok(())
}

#[test]
fn zero_or_missing_measurements_are_not_invented_and_colors_are_optional() -> io::Result<()> {
    for color in [false, true] {
        let output = Output {
            terminal: true,
            color,
        };
        let text = output.render(
            &Command::CacheStatus,
            Format::Table,
            "enabled = true\nmax_entries = 0\nmax_memory_bytes = 0\nmax_ttl_secs = 0\n",
            Path::new("test.sock"),
        )?;
        assert!(text.contains("aucune base de calcul"));
        assert!(text.contains("indisponible"));
        assert!(text.contains("capacité ou TTL nul"));
        assert_eq!(text.contains('\x1b'), color);
    }
    let output = Output {
        terminal: false,
        color: false,
    };
    let text = output.render(
        &Command::Stats,
        Format::Table,
        "requests = 0\n",
        Path::new("bad\x1b[31m\n.sock"),
    )?;
    assert!(text.contains("ancien serveur"));
    assert!(!text.contains('\x1b'));
    assert!(text.contains("bad\\u{1b}[31m\\n.sock"));
    assert!(
        output
            .render(
                &Command::Stats,
                Format::Table,
                "not = [valid",
                Path::new("test.sock")
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn health_describes_exactly_the_checks_requested() -> io::Result<()> {
    let output = Output {
        terminal: true,
        color: false,
    };
    let local = output.render(
        &Command::Health(false),
        Format::Auto,
        "DNS disponible\n",
        Path::new("a.sock"),
    )?;
    assert!(local.contains("UDP et TCP"));
    assert!(!local.contains("upstream"));
    let external = output.render(
        &Command::Health(true),
        Format::Auto,
        "DNS disponible\n",
        Path::new("a.sock"),
    )?;
    assert!(external.contains("résolution upstream"));
    assert_eq!(ratio(Some(1), Some(0)), "— (aucune base de calcul)");
    assert_eq!(
        bytes(u64::MAX),
        "17179869184.00 Gio (18 446 744 073 709 551 615 octets)"
    );
    Ok(())
}
