use prost_types::Timestamp;
use vox_tinvest::generated::v1;
use vox_tinvest::market_data::{
    CanonicalCandle, CanonicalClosePrice, CanonicalMarketValue, CanonicalOrderBook,
    CanonicalTechAnalysisValue, CanonicalTrade, ConfirmationState, MarketDataError,
    MarketDataRequestError, MarketSubscription, MarketSubscriptionRegistry, SubscriptionCommand,
    SubscriptionKind, candle_request_constraint, derived_trade_compatibility_id, lots_to_units,
    merge_historic_candles, plan_candle_history, validate_get_candles_request, validate_ping_delay,
};

fn quotation(units: i64, nano: i32) -> v1::Quotation {
    v1::Quotation { units, nano }
}

#[test]
fn complete_unary_optionality_preserves_missing_as_none() {
    let close = CanonicalClosePrice::try_from(v1::InstrumentClosePriceResponse {
        instrument_uid: "uid".into(),
        ..Default::default()
    })
    .expect("optional close facts");
    assert_eq!(close.price, None);
    assert_eq!(close.evening_session_price, None);
    assert_eq!(close.time_ns, None);

    let tech =
        CanonicalTechAnalysisValue::try_from(v1::get_tech_analysis_response::TechAnalysisItem {
            timestamp: Some(Timestamp {
                seconds: 1,
                nanos: 0,
            }),
            ..Default::default()
        })
        .expect("optional indicator values");
    assert_eq!(tech.middle_band, None);
    assert_eq!(tech.macd, None);

    let market = CanonicalMarketValue::try_from(v1::MarketValue::default())
        .expect("optional market value facts");
    assert_eq!(market.value_type, None);
    assert_eq!(market.value, None);
    assert_eq!(market.time_ns, None);

    let candle = CanonicalCandle::from_historic(
        v1::HistoricCandle {
            open: Some(quotation(1, 0)),
            high: Some(quotation(1, 0)),
            low: Some(quotation(1, 0)),
            close: Some(quotation(1, 0)),
            time: Some(Timestamp {
                seconds: 1,
                nanos: 0,
            }),
            is_complete: true,
            ..Default::default()
        },
        "uid".into(),
        v1::CandleInterval::CandleInterval1Min as i32,
    )
    .expect("request identity must attach explicitly");
    assert_eq!(candle.instrument_uid, "uid");
    assert_eq!(candle.is_complete, Some(true));
}

#[test]
fn generated_trade_normalizes_exact_values_and_lots() {
    let trade = v1::Trade {
        instrument_uid: "uid".into(),
        price: Some(quotation(321, 500_000_000)),
        quantity: 2,
        time: Some(Timestamp {
            seconds: 10,
            nanos: 123,
        }),
        direction: 1,
        trade_source: 2,
        ..Default::default()
    };
    let canonical = CanonicalTrade::try_from(trade).expect("trade must normalize");
    assert_eq!(canonical.price.total_nanos(), 321_500_000_000);
    assert_eq!(canonical.event_time_ns, 10_000_000_123);
    assert_eq!(lots_to_units(canonical.quantity_lots, 10), Ok(20));
    let first = derived_trade_compatibility_id(&canonical);
    assert_eq!(first, derived_trade_compatibility_id(&canonical));
    assert_eq!(first.len(), 35);
    assert!(first.starts_with("TI-"));
}

#[test]
fn missing_economics_and_event_time_fail_closed() {
    let missing_price = v1::Trade {
        instrument_uid: "uid".into(),
        quantity: 1,
        time: Some(Timestamp {
            seconds: 1,
            nanos: 0,
        }),
        ..Default::default()
    };
    assert_eq!(
        CanonicalTrade::try_from(missing_price),
        Err(MarketDataError::Missing("trade.price"))
    );
    assert!(lots_to_units(0, 10).is_err());
    assert!(lots_to_units(1, 0).is_err());
}

#[test]
fn inconsistent_book_fact_is_preserved_but_not_authoritative() {
    let book = v1::OrderBook {
        instrument_uid: "uid".into(),
        depth: 10,
        is_consistent: false,
        time: Some(Timestamp {
            seconds: 10,
            nanos: 0,
        }),
        bids: vec![v1::Order {
            price: Some(quotation(100, 0)),
            quantity: 2,
        }],
        ..Default::default()
    };
    let canonical = CanonicalOrderBook::try_from(book).expect("provider fact must normalize");
    assert!(!canonical.is_consistent);
    assert_eq!(
        canonical.require_authoritative(),
        Err(MarketDataError::InconsistentOrderBook)
    );
}

#[test]
fn history_is_chunked_without_silent_truncation() {
    let day = 86_400;
    let windows = plan_candle_history(0, 3 * day, 1).expect("one-minute interval");
    assert_eq!(windows.len(), 3);
    assert_eq!(windows[0].from_seconds, 0);
    assert_eq!(windows[2].to_seconds, 3 * day);
    assert_eq!(windows[0].limit, 2400);
    assert_eq!(
        candle_request_constraint(15).expect("10 sec").max_limit,
        1250
    );
}

#[test]
fn get_candles_rejects_provider_error_30220_before_dispatch() {
    let mut request = v1::GetCandlesRequest {
        candle_source_type: Some(v1::get_candles_request::CandleSource::Exchange as i32),
        limit: Some(60),
        ..Default::default()
    };
    let error = validate_get_candles_request(&request).expect_err("30220 combination must fail");
    assert_eq!(error, MarketDataRequestError::CandleSourceWithLimit);
    assert_eq!(error.provider_code(), 30_220);

    request.limit = None;
    assert_eq!(validate_get_candles_request(&request), Ok(()));
    request.candle_source_type = None;
    request.limit = Some(60);
    assert_eq!(validate_get_candles_request(&request), Ok(()));
}

#[test]
fn history_merge_deduplicates_exact_boundaries_and_rejects_conflicts() {
    let candle = v1::HistoricCandle {
        open: Some(quotation(1, 0)),
        high: Some(quotation(1, 0)),
        low: Some(quotation(1, 0)),
        close: Some(quotation(1, 0)),
        time: Some(Timestamp {
            seconds: 10,
            nanos: 0,
        }),
        ..Default::default()
    };
    let merged =
        merge_historic_candles([vec![candle], vec![candle]]).expect("exact boundary duplicate");
    assert_eq!(merged.len(), 1);

    let mut conflicting = candle;
    conflicting.close = Some(quotation(2, 0));
    assert_eq!(
        merge_historic_candles([vec![candle], vec![conflicting]]),
        Err(MarketDataError::ConflictingHistoricalCandle {
            timestamp_ns: 10_000_000_000,
        })
    );
}

#[test]
fn registry_accepts_event_before_ack_and_resets_on_disconnect() {
    let subscription = MarketSubscription {
        instrument_id: "uid".into(),
        kind: SubscriptionKind::Trade {
            source: 0,
            with_open_interest: false,
        },
    };
    let mut registry = MarketSubscriptionRegistry::default();
    registry
        .insert(subscription.clone())
        .expect("valid subscription");
    assert!(registry.accepts_event_before_ack("uid", &subscription.kind));
    registry
        .apply_command_response(
            &v1::MarketDataResponse {
                payload: Some(v1::market_data_response::Payload::SubscribeTradesResponse(
                    v1::SubscribeTradesResponse {
                        tracking_id: "tracking".into(),
                        trade_source: 0,
                        trade_subscriptions: vec![v1::TradeSubscription {
                            instrument_uid: "uid".into(),
                            subscription_status: v1::SubscriptionStatus::Success as i32,
                            subscription_action: v1::SubscriptionAction::Subscribe as i32,
                            stream_id: "stream".into(),
                            subscription_id: "00000000-0000-0000-0000-000000000001".into(),
                            ..Default::default()
                        }],
                    },
                )),
            },
            SubscriptionCommand::Subscribe,
        )
        .expect("matching ACK");
    assert!(matches!(
        registry.state(&subscription),
        Some(ConfirmationState::Confirmed { .. })
    ));
    registry.observe_book("uid", true);
    assert!(registry.book_is_authoritative("uid"));
    registry.disconnected();
    assert_eq!(
        registry.state(&subscription),
        Some(&ConfirmationState::Pending)
    );
    assert!(!registry.book_is_authoritative("uid"));
}

#[test]
fn provider_constraints_rejected_before_dispatch() {
    let mut registry = MarketSubscriptionRegistry::default();
    let invalid = MarketSubscription {
        instrument_id: "uid".into(),
        kind: SubscriptionKind::OrderBook {
            depth: 5,
            order_book_type: 0,
        },
    };
    assert_eq!(
        registry.insert(invalid),
        Err(MarketDataError::InvalidOrderBookDepth)
    );
    assert!(validate_ping_delay(4_999).is_err());
    assert!(validate_ping_delay(5_000).is_ok());
    assert!(validate_ping_delay(180_001).is_err());
}

#[test]
#[allow(deprecated)]
fn generated_requests_batch_multi_instrument_subscriptions() {
    let mut registry = MarketSubscriptionRegistry::default();
    for uid in ["uid-a", "uid-b", "uid-c"] {
        registry
            .insert(MarketSubscription {
                instrument_id: uid.into(),
                kind: SubscriptionKind::Trade {
                    source: 0,
                    with_open_interest: false,
                },
            })
            .expect("valid desired subscription");
    }
    let requests = registry.subscribe_requests();
    assert_eq!(requests.len(), 1);
    match requests[0].payload.as_ref() {
        Some(v1::market_data_request::Payload::SubscribeTradesRequest(request)) => {
            assert_eq!(request.instruments.len(), 3);
            assert!(request.instruments.iter().all(|item| item.figi.is_empty()));
        }
        _ => panic!("generated trade subscribe payload required"),
    }
}

#[test]
#[allow(deprecated)]
fn all_five_families_encode_valid_subscribe_requests() {
    let mut registry = MarketSubscriptionRegistry::default();
    for kind in [
        SubscriptionKind::Candle {
            interval: v1::SubscriptionInterval::OneMinute as i32,
            waiting_close: true,
            source: Some(v1::get_candles_request::CandleSource::Exchange as i32),
        },
        SubscriptionKind::OrderBook {
            depth: 10,
            order_book_type: v1::OrderBookType::OrderbookTypeAll as i32,
        },
        SubscriptionKind::Trade {
            source: v1::TradeSourceType::TradeSourceAll as i32,
            with_open_interest: false,
        },
        SubscriptionKind::Info,
        SubscriptionKind::LastPrice,
    ] {
        registry
            .insert(MarketSubscription {
                instrument_id: "uid".into(),
                kind,
            })
            .expect("valid desired subscription");
    }

    let mut families = std::collections::BTreeSet::new();
    for request in registry.subscribe_requests() {
        use v1::market_data_request::Payload;
        match request.payload.expect("generated subscribe payload") {
            Payload::SubscribeCandlesRequest(request) => {
                assert_eq!(request.subscription_action, 1);
                assert_eq!(request.instruments[0].instrument_id, "uid");
                assert_eq!(request.instruments[0].interval, 1);
                assert!(request.waiting_close);
                assert_eq!(request.candle_source_type, Some(1));
                families.insert("candle");
            }
            Payload::SubscribeOrderBookRequest(request) => {
                assert_eq!(request.subscription_action, 1);
                assert_eq!(request.instruments[0].instrument_id, "uid");
                assert_eq!(request.instruments[0].depth, 10);
                assert_eq!(request.instruments[0].order_book_type, 3);
                families.insert("order_book");
            }
            Payload::SubscribeTradesRequest(request) => {
                assert_eq!(request.subscription_action, 1);
                assert_eq!(request.instruments[0].instrument_id, "uid");
                assert_eq!(request.trade_source, 3);
                assert!(!request.with_open_interest);
                families.insert("trade");
            }
            Payload::SubscribeInfoRequest(request) => {
                assert_eq!(request.subscription_action, 1);
                assert_eq!(request.instruments[0].instrument_id, "uid");
                families.insert("info");
            }
            Payload::SubscribeLastPriceRequest(request) => {
                assert_eq!(request.subscription_action, 1);
                assert_eq!(request.instruments[0].instrument_id, "uid");
                families.insert("last_price");
            }
            _ => panic!("unexpected non-subscribe payload"),
        }
    }
    assert_eq!(families.len(), 5);
}

#[test]
fn rejected_ack_fails_without_false_positive_count() {
    let subscription = MarketSubscription {
        instrument_id: "uid".into(),
        kind: SubscriptionKind::LastPrice,
    };
    let mut registry = MarketSubscriptionRegistry::default();
    registry
        .insert(subscription.clone())
        .expect("valid desired subscription");
    let response = v1::MarketDataResponse {
        payload: Some(
            v1::market_data_response::Payload::SubscribeLastPriceResponse(
                v1::SubscribeLastPriceResponse {
                    tracking_id: "track-rejected".into(),
                    last_price_subscriptions: vec![v1::LastPriceSubscription {
                        instrument_uid: "uid".into(),
                        subscription_status: v1::SubscriptionStatus::InstrumentNotFound as i32,
                        subscription_action: v1::SubscriptionAction::Subscribe as i32,
                        ..Default::default()
                    }],
                },
            ),
        ),
    };
    assert_eq!(
        registry.apply_command_response(&response, SubscriptionCommand::Subscribe),
        Err(MarketDataError::SubscriptionRejected {
            family: vox_tinvest::market_data::SubscriptionFamily::LastPrice,
            instrument_uid: "uid".into(),
            provider_status: v1::SubscriptionStatus::InstrumentNotFound as i32,
            tracking_id: "track-rejected".into(),
        })
    );
    assert_eq!(
        registry.state(&subscription),
        Some(&ConfirmationState::Rejected {
            provider_status: v1::SubscriptionStatus::InstrumentNotFound as i32,
        })
    );
    assert!(!registry.all_confirmed());
}

fn last_price_subscription_response(action: v1::SubscriptionAction) -> v1::MarketDataResponse {
    v1::MarketDataResponse {
        payload: Some(
            v1::market_data_response::Payload::SubscribeLastPriceResponse(
                v1::SubscribeLastPriceResponse {
                    tracking_id: "track-control".into(),
                    last_price_subscriptions: vec![v1::LastPriceSubscription {
                        instrument_uid: "uid".into(),
                        subscription_status: v1::SubscriptionStatus::Success as i32,
                        subscription_action: action as i32,
                        stream_id: "stream".into(),
                        subscription_id: "00000000-0000-0000-0000-000000000001".into(),
                        ..Default::default()
                    }],
                },
            ),
        ),
    }
}

#[test]
fn get_my_subscriptions_snapshot_accepts_unspecified_action_and_matches_command_identity() {
    let subscription = MarketSubscription {
        instrument_id: "uid".into(),
        kind: SubscriptionKind::LastPrice,
    };
    let mut registry = MarketSubscriptionRegistry::default();
    registry
        .insert(subscription.clone())
        .expect("valid desired subscription");
    registry
        .apply_command_response(
            &last_price_subscription_response(v1::SubscriptionAction::Subscribe),
            SubscriptionCommand::Subscribe,
        )
        .expect("matching subscribe acknowledgement");

    let snapshots = registry
        .parse_active_snapshot_response(&last_price_subscription_response(
            v1::SubscriptionAction::Unspecified,
        ))
        .expect("authoritative active snapshot");

    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].subscription, subscription);
    assert!(matches!(
        registry.state(&subscription),
        Some(ConfirmationState::Confirmed { .. })
    ));
}

#[test]
fn get_my_subscriptions_snapshot_rejects_changed_provider_identity() {
    let subscription = MarketSubscription {
        instrument_id: "uid".into(),
        kind: SubscriptionKind::LastPrice,
    };
    let mut registry = MarketSubscriptionRegistry::default();
    registry
        .insert(subscription)
        .expect("valid desired subscription");
    registry
        .apply_command_response(
            &last_price_subscription_response(v1::SubscriptionAction::Subscribe),
            SubscriptionCommand::Subscribe,
        )
        .expect("matching subscribe acknowledgement");
    let mut snapshot = last_price_subscription_response(v1::SubscriptionAction::Unspecified);
    let Some(v1::market_data_response::Payload::SubscribeLastPriceResponse(response)) =
        snapshot.payload.as_mut()
    else {
        unreachable!()
    };
    response.last_price_subscriptions[0].subscription_id =
        "00000000-0000-0000-0000-000000000002".into();

    assert_eq!(
        registry.parse_active_snapshot_response(&snapshot),
        Err(MarketDataError::InvalidActiveSnapshotIdentity)
    );
}

#[test]
fn command_ack_requires_context_action_but_supports_unsubscribe() {
    let subscription = MarketSubscription {
        instrument_id: "uid".into(),
        kind: SubscriptionKind::LastPrice,
    };
    let mut registry = MarketSubscriptionRegistry::default();
    registry
        .insert(subscription.clone())
        .expect("valid desired subscription");

    assert_eq!(
        registry.apply_command_response(
            &last_price_subscription_response(v1::SubscriptionAction::Unspecified),
            SubscriptionCommand::Subscribe,
        ),
        Err(MarketDataError::UnexpectedAcknowledgementAction(0))
    );
    registry
        .apply_command_response(
            &last_price_subscription_response(v1::SubscriptionAction::Subscribe),
            SubscriptionCommand::Subscribe,
        )
        .expect("matching subscribe acknowledgement");
    assert!(matches!(
        registry.state(&subscription),
        Some(ConfirmationState::Confirmed { .. })
    ));
    let acknowledgements = registry
        .apply_command_response(
            &last_price_subscription_response(v1::SubscriptionAction::Unsubscribe),
            SubscriptionCommand::Unsubscribe,
        )
        .expect("matching unsubscribe acknowledgement");
    assert_eq!(acknowledgements.len(), 1);
    assert_eq!(
        registry.state(&subscription),
        Some(&ConfirmationState::Pending)
    );
}

#[test]
fn generated_acknowledgements_apply_in_arbitrary_order() {
    let candle = MarketSubscription {
        instrument_id: "uid-candle".into(),
        kind: SubscriptionKind::Candle {
            interval: 1,
            waiting_close: true,
            source: None,
        },
    };
    let trade = MarketSubscription {
        instrument_id: "uid-trade".into(),
        kind: SubscriptionKind::Trade {
            source: 0,
            with_open_interest: false,
        },
    };
    let mut registry = MarketSubscriptionRegistry::default();
    registry.insert(candle.clone()).expect("candle desired");
    registry.insert(trade.clone()).expect("trade desired");

    let trade_ack = v1::MarketDataResponse {
        payload: Some(v1::market_data_response::Payload::SubscribeTradesResponse(
            v1::SubscribeTradesResponse {
                trade_source: 0,
                trade_subscriptions: vec![v1::TradeSubscription {
                    instrument_uid: "uid-trade".into(),
                    subscription_status: v1::SubscriptionStatus::Success as i32,
                    subscription_action: v1::SubscriptionAction::Subscribe as i32,
                    stream_id: "stream".into(),
                    subscription_id: "00000000-0000-0000-0000-000000000001".into(),
                    with_open_interest: false,
                    ..Default::default()
                }],
                ..Default::default()
            },
        )),
    };
    let candle_ack = v1::MarketDataResponse {
        payload: Some(v1::market_data_response::Payload::SubscribeCandlesResponse(
            v1::SubscribeCandlesResponse {
                candles_subscriptions: vec![v1::CandleSubscription {
                    instrument_uid: "uid-candle".into(),
                    interval: 1,
                    waiting_close: true,
                    subscription_status: v1::SubscriptionStatus::Success as i32,
                    subscription_action: v1::SubscriptionAction::Subscribe as i32,
                    stream_id: "stream".into(),
                    subscription_id: "00000000-0000-0000-0000-000000000002".into(),
                    candle_source_type: None,
                    ..Default::default()
                }],
                ..Default::default()
            },
        )),
    };
    assert_eq!(
        registry
            .apply_command_response(&trade_ack, SubscriptionCommand::Subscribe)
            .map(|acknowledgements| acknowledgements.len()),
        Ok(1)
    );
    assert_eq!(
        registry
            .apply_command_response(&candle_ack, SubscriptionCommand::Subscribe)
            .map(|acknowledgements| acknowledgements.len()),
        Ok(1)
    );
    assert!(matches!(
        registry.state(&trade),
        Some(ConfirmationState::Confirmed { .. })
    ));
    assert!(matches!(
        registry.state(&candle),
        Some(ConfirmationState::Confirmed { .. })
    ));
}
