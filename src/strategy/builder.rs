use super::instance::{StrategyInstance, VirtualWallet};
use crate::telegram::TelegramNotifier;
use super::filter::{
    volume::VolumeFilter,
    liquidity::LiquidityFilter,
    blackout::BlackoutFilter,
    spike::SpikeFilter,
    regime::RegimeFilter,
    buyers::BuyersFilter,
    score::{ScoreFilter, ScoreConfig},
    velocity::VelocityFilter,
    TokenFilter,
};
use super::exit::fixed_tp_sl::FixedTpSlExit;
use crate::broker::simulator::RealisticBroker;
use crate::engine::market_context::MarketRegime;

pub struct StrategyBuilder;

impl StrategyBuilder {
    pub fn build_20260515(notifier: Option<TelegramNotifier>) -> StrategyInstance {
        let mut filters: Vec<Box<dyn TokenFilter>> = Vec::new();
        filters.push(Box::new(VolumeFilter { min_volume_sol: 3.0 })); // Anggap default thresh 3.0
        filters.push(Box::new(BuyersFilter { min_buyers: 6 }));
        filters.push(Box::new(ScoreFilter {
            config: ScoreConfig {
                volume_thresh: 3.0,
                buyers_thresh: 6,
                velocity_thresh: 1.5,
                min_score: 65.0,
                enable_early_acceleration_bias: false,
            }
        }));

        StrategyInstance {
            strategy_id: "Alpha".to_string(),
            filters,
            broker: Box::new(RealisticBroker {
                trading_fee_rate: 0.0,
                priority_fee_sol: 0.0,
                slippage_tp: 0.0,
                slippage_sl: 0.0,
                net_roi_enabled: false,
            }),
            exit: Box::new(FixedTpSlExit {
                tp_multiplier: 1.15, // +15%
                sl_multiplier: 0.92, // -8%
                timeout_secs: 120,
            }),
            wallet: VirtualWallet::default(),
            notifier,
            open_positions: std::collections::HashMap::new(),
            bootstrap_enabled: false,
        }
    }

    pub fn build_20260517(notifier: Option<TelegramNotifier>) -> StrategyInstance {
        let mut filters: Vec<Box<dyn TokenFilter>> = Vec::new();
        filters.push(Box::new(VolumeFilter { min_volume_sol: 3.0 }));
        filters.push(Box::new(BuyersFilter { min_buyers: 6 }));
        filters.push(Box::new(ScoreFilter {
            config: ScoreConfig {
                volume_thresh: 3.0,
                buyers_thresh: 6,
                velocity_thresh: 1.5,
                min_score: 60.0, // diturunkan ke 60
                enable_early_acceleration_bias: true, // bias aktif
            }
        }));

        StrategyInstance {
            strategy_id: "Bravo".to_string(),
            filters,
            broker: Box::new(RealisticBroker {
                trading_fee_rate: 0.0,
                priority_fee_sol: 0.0,
                slippage_tp: 0.0,
                slippage_sl: 0.0,
                net_roi_enabled: false,
            }),
            exit: Box::new(FixedTpSlExit {
                tp_multiplier: 1.15,
                sl_multiplier: 0.92,
                timeout_secs: 120,
            }),
            wallet: VirtualWallet::default(),
            notifier,
            open_positions: std::collections::HashMap::new(),
            bootstrap_enabled: true, // aktif
        }
    }

    pub fn build_20260523(notifier: Option<TelegramNotifier>) -> StrategyInstance {
        let mut instance = Self::build_20260517(notifier);
        instance.strategy_id = "Charlie".to_string();
        
        // Update Exit
        instance.exit = Box::new(FixedTpSlExit {
            tp_multiplier: 1.30, // +30%
            sl_multiplier: 0.92, // -8%
            timeout_secs: 120,
        });

        // Update Broker (Net ROI)
        instance.broker = Box::new(RealisticBroker {
            trading_fee_rate: 0.0125, // 1.25%
            priority_fee_sol: 0.002,
            slippage_tp: 0.01, // 1%
            slippage_sl: 0.03, // 3%
            net_roi_enabled: true,
        });

        // Add Blackout
        instance.filters.insert(0, Box::new(BlackoutFilter {
            blackout_hours: vec![7, 12, 19],
            blackout_window_minutes: 60,
        }));

        instance.filters.push(Box::new(VelocityFilter {
            min_velocity_sol: 1.5,
        }));

        instance
    }

    pub fn build_20260524(notifier: Option<TelegramNotifier>) -> StrategyInstance {
        let mut instance = Self::build_20260523(notifier);
        instance.strategy_id = "Delta".to_string();

        instance.filters.insert(0, Box::new(LiquidityFilter { min_liquidity_sol: 10.0 }));
        instance.filters.insert(1, Box::new(SpikeFilter { max_multiplier: 3.0 }));

        instance
    }

    pub fn build_20260527(notifier: Option<TelegramNotifier>) -> StrategyInstance {
        let mut instance = Self::build_20260524(notifier);
        instance.strategy_id = "Echo".to_string();

        // Hanya boleh jalan di Hot Market
        instance.filters.insert(0, Box::new(RegimeFilter { 
            required_regimes: vec![MarketRegime::Hot] 
        }));

        instance
    }

    // Builder utama yang menjalankan semua
    pub fn build_all(notifier: Option<TelegramNotifier>) -> Vec<StrategyInstance> {
        vec![
            Self::build_20260515(notifier.clone()),
            Self::build_20260517(notifier.clone()),
            Self::build_20260523(notifier.clone()),
            Self::build_20260524(notifier.clone()),
            Self::build_20260527(notifier),
        ]
    }
}
