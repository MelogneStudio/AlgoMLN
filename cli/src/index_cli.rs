use std::path::PathBuf;

use algomln_data::{
    default_indices_cache_dir, refresh_all_indices, refresh_index, Catalog, IndexAlias,
    IndexRegistry, RefreshError,
};

pub fn main_entry(args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    let Some(sub) = args.first().cloned() else {
        return Err("usage: algomln index <list|show|refresh> [options]".into());
    };

    let mut opts = std::collections::BTreeMap::new();
    let mut positional = Vec::new();
    let mut it = args.into_iter().skip(1);
    while let Some(k) = it.next() {
        if k.starts_with("--") {
            if matches!(k.as_str(), "--dry-run" | "--allow-unresolved") {
                opts.insert(k, "true".into());
                continue;
            }
            let val = it.next().ok_or_else(|| format!("missing value for {k}"))?;
            opts.insert(k, val);
        } else {
            positional.push(k);
        }
    }

    let data_dir = opts
        .get("--data")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data"));

    let cache_dir = if opts.contains_key("--data") {
        data_dir.join("indices")
    } else {
        default_indices_cache_dir()
    };

    let registry = IndexRegistry::load_with_cache_dir(Some(cache_dir.clone()));

    match sub.as_str() {
        "list" => {
            let meta = registry.list_metadata();
            println!(
                "{:<26} {:<26} {:<26} {:>8}  {:<12}",
                "ALIAS", "DISPLAY NAME", "DSL KEYWORD", "SYMBOLS", "AS OF"
            );
            println!("{}", "-".repeat(102));
            for m in meta {
                println!(
                    "{:<26} {:<26} {:<26} {:>8}  {:<12}",
                    m.alias, m.display_name, m.dsl_keyword, m.symbol_count, m.as_of
                );
            }
            Ok(())
        }
        "show" => {
            let target = positional
                .first()
                .ok_or("usage: algomln index show <alias>")?;
            let alias = IndexAlias::from_keyword(target)
                .ok_or_else(|| format!("unknown index alias '{target}'"))?;

            let entry = registry
                .get_entry(alias)
                .ok_or_else(|| format!("no registry entry for {}", alias.display_name()))?;

            println!("Index:        {}", entry.display_name);
            println!("DSL Keyword:  {}", entry.dsl_keyword);
            println!("Alias:        {}", entry.alias.alias());
            println!("As of:        {}", entry.as_of);
            if let Some(url) = &entry.source_url {
                println!("Source URL:   {}", url);
            }
            println!("CSV Filename: {}", alias.csv_filename());

            let db_path = data_dir.join("catalog.sqlite");
            let cat = Catalog::open_read_only(&db_path).ok();

            println!("\nConstituents ({} total):", entry.symbols.len());
            let mut resolved_count = 0;
            let mut unresolved_count = 0;

            for sym in &entry.symbols {
                match &cat {
                    Some(c) => match c.resolve_full(sym) {
                        Ok(inst) => {
                            resolved_count += 1;
                            println!(
                                "  [RESOLVED]   {:<12} (SecID: {:<6}, F&O: {:<5}, Lot: {:>4}, Tick: {:>2}p)",
                                sym, inst.security_id, inst.is_fno, inst.lot_size, inst.tick_size_paise
                            );
                        }
                        Err(_) => {
                            unresolved_count += 1;
                            println!("  [UNRESOLVED] {:<12} (not found in catalog)", sym);
                        }
                    },
                    None => {
                        println!("  {}", sym);
                    }
                }
            }

            if cat.is_some() {
                println!(
                    "\nSummary: {} resolved against scrip catalog, {} unresolved",
                    resolved_count, unresolved_count
                );
            }

            Ok(())
        }
        "refresh" => {
            let maybe_target = positional.first();
            match maybe_target {
                Some(target) => {
                    let alias = IndexAlias::from_keyword(target)
                        .ok_or_else(|| format!("unknown index alias '{target}'"))?;
                    println!(
                        "Refreshing {} ({}) from niftyindices.com...",
                        alias.display_name(),
                        alias.csv_filename()
                    );
                    match refresh_index(alias, &cache_dir, Some(&registry), None, None) {
                        Ok(file) => {
                            println!(
                                "✓ Successfully refreshed {}: {} symbols (as of {})",
                                alias.display_name(),
                                file.symbols.len(),
                                file.as_of
                            );
                        }
                        Err(e) => {
                            match e {
                                RefreshError::HttpStatus { status, url } => {
                                    eprintln!("✗ HTTP {status} fetching {url}");
                                }
                                RefreshError::Parse { alias, message } => {
                                    eprintln!("✗ Parse error for {alias}: {message}");
                                }
                                other => {
                                    eprintln!("✗ Refresh failed: {other}");
                                }
                            }
                            return Err("index refresh failed".into());
                        }
                    }
                }
                None => {
                    println!("Refreshing all 22 indices from niftyindices.com...");
                    let outcomes = refresh_all_indices(&registry, &cache_dir, None, None);
                    let mut succeeded = 0;
                    let mut failed = 0;

                    for outcome in &outcomes {
                        if outcome.success {
                            succeeded += 1;
                            println!(
                                "  ✓ {:<26} {:>4} symbols (as of {})",
                                outcome.display_name, outcome.symbol_count, outcome.as_of
                            );
                        } else {
                            failed += 1;
                            println!(
                                "  ✗ {:<26} FAILED: {}",
                                outcome.display_name,
                                outcome.error.as_deref().unwrap_or("unknown error")
                            );
                        }
                    }

                    println!("\nRefresh summary: {succeeded} succeeded, {failed} failed (total {})", outcomes.len());
                    if failed > 0 {
                        eprintln!("Note: Failed indices left existing cached constituents untouched.");
                    }
                }
            }
            Ok(())
        }
        other => Err(format!("unknown index subcommand '{other}': use list, show, or refresh").into()),
    }
}
