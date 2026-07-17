use chrono::NaiveTime;
use serde::{Deserialize, Serialize};

/// Named NSE index alias. Resolved to a constituent symbol list at runtime
/// via `IndexRegistry` (loaded once at deploy time, read-only afterwards).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IndexAlias {
    Nifty50,
    NiftyNext50,
    Nifty100,
    Nifty200,
    Nifty500,
    NiftyMidcap50,
    NiftyMidcap100,
    NiftyMidcap150,
    NiftySmallcap50,
    NiftySmallcap100,
    NiftySmallcap250,
    NiftyBank,
    NiftyIt,
    NiftyPharma,
    NiftyAuto,
    NiftyFmcg,
    NiftyMetal,
    NiftyRealty,
    NiftyEnergy,
    NiftyInfra,
    NiftyPsuBank,
    NiftyFinancialServices,
}

impl IndexAlias {
    /// Parse from the DSL keyword string (case-insensitive, underscore-separated).
    pub fn from_dsl_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "NIFTY_50" => Some(Self::Nifty50),
            "NIFTY_NEXT_50" => Some(Self::NiftyNext50),
            "NIFTY_100" => Some(Self::Nifty100),
            "NIFTY_200" => Some(Self::Nifty200),
            "NIFTY_500" => Some(Self::Nifty500),
            "NIFTY_MIDCAP_50" => Some(Self::NiftyMidcap50),
            "NIFTY_MIDCAP_100" => Some(Self::NiftyMidcap100),
            "NIFTY_MIDCAP_150" => Some(Self::NiftyMidcap150),
            "NIFTY_SMALLCAP_50" => Some(Self::NiftySmallcap50),
            "NIFTY_SMALLCAP_100" => Some(Self::NiftySmallcap100),
            "NIFTY_SMALLCAP_250" => Some(Self::NiftySmallcap250),
            "NIFTY_BANK" => Some(Self::NiftyBank),
            "NIFTY_IT" => Some(Self::NiftyIt),
            "NIFTY_PHARMA" => Some(Self::NiftyPharma),
            "NIFTY_AUTO" => Some(Self::NiftyAuto),
            "NIFTY_FMCG" => Some(Self::NiftyFmcg),
            "NIFTY_METAL" => Some(Self::NiftyMetal),
            "NIFTY_REALTY" => Some(Self::NiftyRealty),
            "NIFTY_ENERGY" => Some(Self::NiftyEnergy),
            "NIFTY_INFRA" => Some(Self::NiftyInfra),
            "NIFTY_PSU_BANK" => Some(Self::NiftyPsuBank),
            "NIFTY_FINANCIAL_SERVICES" => Some(Self::NiftyFinancialServices),
            _ => None,
        }
    }

    /// Human-readable display name.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Nifty50 => "NIFTY 50",
            Self::NiftyNext50 => "NIFTY NEXT 50",
            Self::Nifty100 => "NIFTY 100",
            Self::Nifty200 => "NIFTY 200",
            Self::Nifty500 => "NIFTY 500",
            Self::NiftyMidcap50 => "NIFTY MIDCAP 50",
            Self::NiftyMidcap100 => "NIFTY MIDCAP 100",
            Self::NiftyMidcap150 => "NIFTY MIDCAP 150",
            Self::NiftySmallcap50 => "NIFTY SMALLCAP 50",
            Self::NiftySmallcap100 => "NIFTY SMALLCAP 100",
            Self::NiftySmallcap250 => "NIFTY SMALLCAP 250",
            Self::NiftyBank => "NIFTY BANK",
            Self::NiftyIt => "NIFTY IT",
            Self::NiftyPharma => "NIFTY PHARMA",
            Self::NiftyAuto => "NIFTY AUTO",
            Self::NiftyFmcg => "NIFTY FMCG",
            Self::NiftyMetal => "NIFTY METAL",
            Self::NiftyRealty => "NIFTY REALTY",
            Self::NiftyEnergy => "NIFTY ENERGY",
            Self::NiftyInfra => "NIFTY INFRA",
            Self::NiftyPsuBank => "NIFTY PSU BANK",
            Self::NiftyFinancialServices => "NIFTY FINANCIAL SERVICES",
        }
    }

    /// DSL keyword form (what the user writes after TRADE_IN).
    pub fn dsl_keyword(&self) -> &'static str {
        match self {
            Self::Nifty50 => "NIFTY_50",
            Self::NiftyNext50 => "NIFTY_NEXT_50",
            Self::Nifty100 => "NIFTY_100",
            Self::Nifty200 => "NIFTY_200",
            Self::Nifty500 => "NIFTY_500",
            Self::NiftyMidcap50 => "NIFTY_MIDCAP_50",
            Self::NiftyMidcap100 => "NIFTY_MIDCAP_100",
            Self::NiftyMidcap150 => "NIFTY_MIDCAP_150",
            Self::NiftySmallcap50 => "NIFTY_SMALLCAP_50",
            Self::NiftySmallcap100 => "NIFTY_SMALLCAP_100",
            Self::NiftySmallcap250 => "NIFTY_SMALLCAP_250",
            Self::NiftyBank => "NIFTY_BANK",
            Self::NiftyIt => "NIFTY_IT",
            Self::NiftyPharma => "NIFTY_PHARMA",
            Self::NiftyAuto => "NIFTY_AUTO",
            Self::NiftyFmcg => "NIFTY_FMCG",
            Self::NiftyMetal => "NIFTY_METAL",
            Self::NiftyRealty => "NIFTY_REALTY",
            Self::NiftyEnergy => "NIFTY_ENERGY",
            Self::NiftyInfra => "NIFTY_INFRA",
            Self::NiftyPsuBank => "NIFTY_PSU_BANK",
            Self::NiftyFinancialServices => "NIFTY_FINANCIAL_SERVICES",
        }
    }

    /// niftyindices.com CSV filename for this index.
    pub fn csv_filename(&self) -> &'static str {
        match self {
            Self::Nifty50 => "ind_nifty50list.csv",
            Self::NiftyNext50 => "ind_niftynext50list.csv",
            Self::Nifty100 => "ind_nifty100list.csv",
            Self::Nifty200 => "ind_nifty200list.csv",
            Self::Nifty500 => "ind_nifty500list.csv",
            Self::NiftyMidcap50 => "ind_niftymidcap50list.csv",
            Self::NiftyMidcap100 => "ind_niftymidcap100list.csv",
            Self::NiftyMidcap150 => "ind_niftymidcap150list.csv",
            Self::NiftySmallcap50 => "ind_niftysmallcap50list.csv",
            Self::NiftySmallcap100 => "ind_niftysmallcap100list.csv",
            Self::NiftySmallcap250 => "ind_niftysmallcap250list.csv",
            Self::NiftyBank => "ind_niftybanklist.csv",
            Self::NiftyIt => "ind_niftyitlist.csv",
            Self::NiftyPharma => "ind_niftypharmaList.csv",
            Self::NiftyAuto => "ind_niftyautolist.csv",
            Self::NiftyFmcg => "ind_niftyfmcglist.csv",
            Self::NiftyMetal => "ind_niftymetallist.csv",
            Self::NiftyRealty => "ind_niftyrealtylist.csv",
            Self::NiftyEnergy => "ind_niftyenergylist.csv",
            Self::NiftyInfra => "ind_niftyinfraList.csv",
            Self::NiftyPsuBank => "ind_niftypsubanklist.csv",
            Self::NiftyFinancialServices => "ind_niftyfinancialserviceslist.csv",
        }
    }

    /// Lowercase filename stem used for JSON cache files (e.g. "nifty_50").
    pub fn cache_stem(&self) -> &'static str {
        match self {
            Self::Nifty50 => "nifty_50",
            Self::NiftyNext50 => "nifty_next_50",
            Self::Nifty100 => "nifty_100",
            Self::Nifty200 => "nifty_200",
            Self::Nifty500 => "nifty_500",
            Self::NiftyMidcap50 => "nifty_midcap_50",
            Self::NiftyMidcap100 => "nifty_midcap_100",
            Self::NiftyMidcap150 => "nifty_midcap_150",
            Self::NiftySmallcap50 => "nifty_smallcap_50",
            Self::NiftySmallcap100 => "nifty_smallcap_100",
            Self::NiftySmallcap250 => "nifty_smallcap_250",
            Self::NiftyBank => "nifty_bank",
            Self::NiftyIt => "nifty_it",
            Self::NiftyPharma => "nifty_pharma",
            Self::NiftyAuto => "nifty_auto",
            Self::NiftyFmcg => "nifty_fmcg",
            Self::NiftyMetal => "nifty_metal",
            Self::NiftyRealty => "nifty_realty",
            Self::NiftyEnergy => "nifty_energy",
            Self::NiftyInfra => "nifty_infra",
            Self::NiftyPsuBank => "nifty_psu_bank",
            Self::NiftyFinancialServices => "nifty_financial_services",
        }
    }

    /// All variants in a stable order. Used for iteration.
    pub fn all() -> &'static [IndexAlias] {
        use IndexAlias::*;
        &[
            Nifty50, NiftyNext50, Nifty100, Nifty200, Nifty500, NiftyMidcap50, NiftyMidcap100,
            NiftyMidcap150, NiftySmallcap50, NiftySmallcap100, NiftySmallcap250, NiftyBank,
            NiftyIt, NiftyPharma, NiftyAuto, NiftyFmcg, NiftyMetal, NiftyRealty, NiftyEnergy,
            NiftyInfra, NiftyPsuBank, NiftyFinancialServices,
        ]
    }
}

/// What a strategy trades over. `None` on `StrategyNode` means single-symbol
/// (backwards compatible); the engine falls back to the explicit `symbol`
/// argument it was constructed with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TradeIn {
    /// Explicit comma-separated list of NSE symbols (e.g. `TRADE_IN RELIANCE, INFY`).
    Symbols(Vec<String>),
    /// Named index alias (e.g. `TRADE_IN NIFTY_BANK`).
    Index(IndexAlias),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyNode {
    pub name: String,
    /// `None` for the existing single-symbol flow. The engine reads this once
    /// at deploy time to fan the ruleset out over a symbol universe.
    pub trade_in: Option<TradeIn>,
    pub rules: Vec<RuleNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleNode {
    pub id: String,
    pub condition: ConditionNode,
    pub action: ActionNode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConditionNode {
    Comparison {
        left: ExprNode,
        op: CompareOp,
        right: ExprNode,
    },
    CrossAbove {
        fast: ExprNode,
        slow: ExprNode,
    },
    CrossBelow {
        fast: ExprNode,
        slow: ExprNode,
    },
    And(Box<ConditionNode>, Box<ConditionNode>),
    Or(Box<ConditionNode>, Box<ConditionNode>),
    Not(Box<ConditionNode>),
    InPosition,
    TimeWindow {
        start: NaiveTime,
        end: NaiveTime,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExprNode {
    Indicator(IndicatorCall),
    PriceField(PriceField),
    Literal(f64),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndicatorCall {
    pub kind: IndicatorKind,
    pub period: usize,
}

/// Indicator functions available in DSL expressions:
/// ema(N), ma(N), rsi(N), rel_vol(N), atr(N), vwap(N), bb_upper(N),
/// bb_lower(N), bb_mid(N).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IndicatorKind {
    Ema,
    Ma,
    Rsi,
    RelVol,
    Atr,
    Vwap,
    BbUpper,
    BbLower,
    BbMid,
}

/// Candle price fields available in DSL expressions:
/// close, open, high, low, volume, prev_close.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PriceField {
    Close,
    Open,
    High,
    Low,
    Volume,
    PrevClose,
    PrevOpen,
    PrevHigh,
    PrevLow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompareOp {
    Lt,
    Gt,
    Lte,
    Gte,
    Eq,
    Neq,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActionNode {
    Buy { quantity: usize },
    Sell { quantity: usize },
    SellAll,
}
