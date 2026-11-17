use crate::error::ApiError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scrip {
    pub security_id: String,
    pub symbol: String,
    pub lot_size: u32,
    pub tick_size_paise: i64,
    pub instrument: String,
    pub is_fno: bool,
}

impl Scrip {
    /// Expected 1-minute bars per regular session.
    ///
    /// Since NSE's closing auction session began on 2026-08-03:
    /// - F&O-eligible stocks trade until 15:14 (360 bars: 09:15 to 15:14).
    /// - Non-F&O stocks still trade until 15:29 (375 bars: 09:15 to 15:29).
    pub fn expected_1m_bars_per_session(&self) -> u32 {
        if self.is_fno {
            360
        } else {
            375
        }
    }
}

/// Parses Dhan's detailed scrip master CSV.
///
/// Data source: <https://images.dhan.co/api-data/api-scrip-master-detailed.csv>
pub fn parse_scrip_master<R: std::io::Read>(reader: R) -> Result<Vec<Scrip>, ApiError> {
    let mut csv = csv::ReaderBuilder::new().flexible(true).from_reader(reader);
    let headers = csv
        .headers()
        .map_err(|e| ApiError::Malformed(e.to_string()))?
        .clone();
    let idx = |n: &str| {
        headers
            .iter()
            .position(|x| x == n)
            .ok_or_else(|| ApiError::Malformed(format!("missing scrip column {n}")))
    };
    let exchange = idx("EXCH_ID")?;
    let security_id = idx("SECURITY_ID")?;
    let underlying_security_id = idx("UNDERLYING_SECURITY_ID")?;
    let symbol = idx("UNDERLYING_SYMBOL")?;
    let instrument = idx("INSTRUMENT")?;
    let instrument_type = idx("INSTRUMENT_TYPE")?;
    let series = idx("SERIES")?;
    let lot = idx("LOT_SIZE")?;
    let tick = idx("TICK_SIZE")?;

    let mut equities = Vec::new();
    let mut fno_ids = std::collections::HashSet::new();
    let mut fno_symbols = std::collections::HashSet::new();

    for row in csv.records() {
        let r = row.map_err(|e| ApiError::Malformed(e.to_string()))?;
        let exch = r.get(exchange).unwrap_or("");
        let inst = r.get(instrument).unwrap_or("");

        if exch == "NSE" && (inst == "FUTSTK" || inst == "OPTSTK") {
            if let Some(und_id) = r.get(underlying_security_id) {
                let s = und_id.trim();
                if !s.is_empty() {
                    fno_ids.insert(s.to_string());
                }
            }
            if let Some(und_sym) = r.get(symbol) {
                let s = und_sym.trim();
                if !s.is_empty() {
                    fno_symbols.insert(s.to_ascii_uppercase());
                }
            }
        }

        if exch == "NSE"
            && r.get(series).is_some_and(|s| s == "EQ")
            && r.get(instrument_type).is_some_and(|s| s == "ES")
            && inst == "EQUITY"
        {
            let sec_id = r
                .get(security_id)
                .ok_or_else(|| ApiError::Malformed("missing SECURITY_ID in record".into()))?
                .trim();
            let sym = r
                .get(symbol)
                .ok_or_else(|| ApiError::Malformed("missing UNDERLYING_SYMBOL in record".into()))?
                .trim();
            let raw: f64 = r
                .get(tick)
                .ok_or_else(|| ApiError::Malformed("missing TICK_SIZE in record".into()))?
                .parse()
                .map_err(|_| ApiError::Malformed("invalid TICK_SIZE".into()))?;
            let paise = raw.round() as i64;
            let lot_raw: f64 = r
                .get(lot)
                .ok_or_else(|| ApiError::Malformed("missing LOT_SIZE in record".into()))?
                .parse()
                .map_err(|_| ApiError::Malformed("invalid LOT_SIZE".into()))?;
            if !lot_raw.is_finite()
                || lot_raw < 1.0
                || lot_raw.fract() != 0.0
                || lot_raw > u32::MAX as f64
            {
                return Err(ApiError::Malformed(
                    "LOT_SIZE must be a positive whole number".into(),
                ));
            }
            if paise <= 0 {
                return Err(ApiError::Malformed(
                    "TICK_SIZE must be positive paise".into(),
                ));
            }
            equities.push(Scrip {
                security_id: sec_id.into(),
                symbol: sym.to_ascii_uppercase(),
                lot_size: lot_raw as u32,
                tick_size_paise: paise,
                instrument: inst.to_string(),
                is_fno: false,
            });
        }
    }

    for scrip in &mut equities {
        if fno_ids.contains(&scrip.security_id) || fno_symbols.contains(&scrip.symbol) {
            scrip.is_fno = true;
        }
    }

    Ok(equities)
}

#[derive(Debug)]
pub struct ScripResolver {
    by_symbol: std::collections::HashMap<String, Scrip>,
}

impl ScripResolver {
    pub fn new(rows: Vec<Scrip>) -> Result<Self, ApiError> {
        let mut by_symbol = std::collections::HashMap::new();
        for x in rows {
            if by_symbol.insert(x.symbol.clone(), x.clone()).is_some() {
                return Err(ApiError::Malformed(format!(
                    "duplicate NSE symbol {}",
                    x.symbol
                )));
            }
        }
        Ok(Self { by_symbol })
    }

    pub fn resolve(&self, s: &str) -> Result<&Scrip, ApiError> {
        self.by_symbol
            .get(&s.trim().to_ascii_uppercase())
            .ok_or_else(|| ApiError::DataApi {
                code: "UNKNOWN_SYMBOL".into(),
                message: s.into(),
            })
    }
}
