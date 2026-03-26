use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum AutoTradeError {
    InvalidAmount = 1,
    Unauthorized = 2,
    SignalNotFound = 3,
    SignalExpired = 4,
    InsufficientBalance = 5,
    InsufficientLiquidity = 6,
    DailyTradeLimitExceeded = 7,
    PositionLimitExceeded = 8,
    StopLossTriggered = 9,
    DcaStrategyNotFound = 10,
    DcaStrategyInactive = 11,
    DcaEndTimeReached = 12,
    MrStrategyNotFound = 13,
    MrInsufficientHistory = 14,
    MrLowVolatility = 15,
}
