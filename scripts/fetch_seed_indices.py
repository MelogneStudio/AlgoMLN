#!/usr/bin/env python3
"""
Fetch seed index constituent data from niftyindices.com.

Run once from repo root:

    python scripts/fetch_seed_indices.py

Outputs JSON to: src-tauri/resources/indices/

Requires: Python 3.8+, no external dependencies (stdlib only).
"""

import csv
import io
import json
import urllib.request
from datetime import date
from pathlib import Path

INDICES = [
    ("NIFTY_50",                "nifty_50",                "ind_nifty50list.csv"),
    ("NIFTY_NEXT_50",           "nifty_next_50",           "ind_niftynext50list.csv"),
    ("NIFTY_100",               "nifty_100",               "ind_nifty100list.csv"),
    ("NIFTY_200",               "nifty_200",               "ind_nifty200list.csv"),
    ("NIFTY_500",               "nifty_500",               "ind_nifty500list.csv"),
    ("NIFTY_MIDCAP_50",         "nifty_midcap_50",         "ind_niftymidcap50list.csv"),
    ("NIFTY_MIDCAP_100",        "nifty_midcap_100",        "ind_niftymidcap100list.csv"),
    ("NIFTY_MIDCAP_150",        "nifty_midcap_150",        "ind_niftymidcap150list.csv"),
    ("NIFTY_SMALLCAP_50",       "nifty_smallcap_50",       "ind_niftysmallcap50list.csv"),
    ("NIFTY_SMALLCAP_100",      "nifty_smallcap_100",      "ind_niftysmallcap100list.csv"),
    ("NIFTY_SMALLCAP_250",      "nifty_smallcap_250",      "ind_niftysmallcap250list.csv"),
    ("NIFTY_BANK",              "nifty_bank",              "ind_niftybanklist.csv"),
    ("NIFTY_IT",                "nifty_it",                "ind_niftyitlist.csv"),
    ("NIFTY_PHARMA",            "nifty_pharma",            "ind_niftypharmaList.csv"),
    ("NIFTY_AUTO",              "nifty_auto",              "ind_niftyautolist.csv"),
    ("NIFTY_FMCG",              "nifty_fmcg",              "ind_niftyfmcglist.csv"),
    ("NIFTY_METAL",             "nifty_metal",             "ind_niftymetallist.csv"),
    ("NIFTY_REALTY",            "nifty_realty",            "ind_niftyrealtylist.csv"),
    ("NIFTY_ENERGY",            "nifty_energy",            "ind_niftyenergylist.csv"),
    ("NIFTY_INFRA",             "nifty_infra",             "ind_niftyinfraList.csv"),
    ("NIFTY_PSU_BANK",          "nifty_psu_bank",          "ind_niftypsubanklist.csv"),
    ("NIFTY_FINANCIAL_SERVICES","nifty_financial_services","ind_niftyfinancialserviceslist.csv"),
]

BASE_URL = "https://www.niftyindices.com/IndexConstituent/"
OUTPUT_DIR = Path("src-tauri/resources/indices")
OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
TODAY = date.today().isoformat()
HEADERS = {"User-Agent": "Mozilla/5.0 (AlgoMLN seed fetcher)"}


def fetch(alias, stem, filename):
    url = BASE_URL + filename
    try:
        req = urllib.request.Request(url, headers=HEADERS)
        with urllib.request.urlopen(req, timeout=15) as resp:
            raw = resp.read().decode("utf-8-sig")  # strip BOM
        reader = csv.DictReader(io.StringIO(raw))
        symbols = [
            row["Symbol"].strip().upper()
            for row in reader
            if row.get("Symbol", "").strip()
        ]
        if not symbols:
            raise ValueError("no symbols found in CSV")
        data = {"alias": alias, "last_updated": TODAY, "symbols": symbols}
        (OUTPUT_DIR / f"{stem}.json").write_text(json.dumps(data, indent=2))
        print(f"  ✓  {alias:<30} {len(symbols):>4} symbols")
        return True
    except Exception as e:
        print(f"  ✗  {alias:<30} FAILED: {e}")
        # Write empty placeholder so the file exists and Rust doesn't error
        # on first boot before the user has run a successful refresh.
        data = {"alias": alias, "last_updated": "never", "symbols": []}
        (OUTPUT_DIR / f"{stem}.json").write_text(json.dumps(data, indent=2))
        return False


def main():
    print(f"Fetching {len(INDICES)} indices from niftyindices.com…\n")
    ok = sum(fetch(*row) for row in INDICES)
    print(f"\n{ok}/{len(INDICES)} indices fetched successfully.")
    print(f"Output: {OUTPUT_DIR.resolve()}")
    if ok < len(INDICES):
        print("WARNING: Some indices failed. The app will show 0 symbols for those until")
        print("the user clicks 'Refresh Indices' in Settings while connected to the internet.")


if __name__ == "__main__":
    main()
