use super::*;
use serde_json::json;

fn run(script: &str) -> Result<EvaluateScriptResponse, EvalFailure> {
    evaluate(EvaluateScriptArgs {
        script: script.to_string(),
        input: Value::Null,
        ..EvaluateScriptArgs::default()
    })
}

#[test]
fn evaluates_json_input_without_state_leakage() {
    let first = evaluate(EvaluateScriptArgs {
        script: "input[\"count\"] + 1".to_string(),
        input: json!({"count": 2}),
        ..EvaluateScriptArgs::default()
    })
    .expect("first evaluation should succeed");
    let second = run("40 + 2").expect("second evaluation should succeed");

    assert_eq!(first.value, json!(3));
    assert_eq!(second.value, json!(42));
    assert_eq!(first.output, "");
}

#[test]
fn map_serialization_is_stable() {
    let response = run("#{z: 1, a: 2}").expect("map should evaluate");
    assert_eq!(
        serde_json::to_string(&response.value).unwrap(),
        r#"{"a":2,"z":1}"#
    );
}

#[test]
fn profiles_are_listed_in_stable_order() {
    let names: Vec<_> = ProfileName::ALL
        .into_iter()
        .map(ProfileName::as_str)
        .collect();
    assert_eq!(names, vec!["default", "restricted", "surface"]);
    assert!(
        profile_defaults(ProfileName::Restricted)
            .limits
            .max_operations
            < HARD_MAX_OPERATIONS
    );
}

#[test]
fn requests_can_only_narrow_limits() {
    let narrowed = evaluate(EvaluateScriptArgs {
        script: "1 + 1".to_string(),
        limits: Some(RequestedLimits {
            max_operations: Some(100),
            ..RequestedLimits::default()
        }),
        ..EvaluateScriptArgs::default()
    })
    .expect("lower operation limit should be accepted");
    assert_eq!(narrowed.limits.max_operations, 100);

    let widened = evaluate(EvaluateScriptArgs {
        script: "1 + 1".to_string(),
        limits: Some(RequestedLimits {
            max_operations: Some(HARD_MAX_OPERATIONS),
            ..RequestedLimits::default()
        }),
        ..EvaluateScriptArgs::default()
    })
    .expect_err("a request above the default profile must be rejected");
    assert_eq!(widened.code.as_str(), "limit_widened");
}

#[test]
fn requests_cannot_reenable_disabled_mechanics() {
    let error = evaluate(EvaluateScriptArgs {
        script: "1".to_string(),
        profile: Some(ProfileName::Restricted),
        features: Some(RequestedFeatures {
            allow_loops: Some(true),
            ..RequestedFeatures::default()
        }),
        ..EvaluateScriptArgs::default()
    })
    .expect_err("restricted loops cannot be re-enabled");
    assert_eq!(error.code.as_str(), "feature_widened");

    let error = evaluate(EvaluateScriptArgs {
        script: "let x = 1; x".to_string(),
        profile: Some(ProfileName::Default),
        features: Some(RequestedFeatures {
            allow_loops: Some(false),
            ..RequestedFeatures::default()
        }),
        ..EvaluateScriptArgs::default()
    })
    .expect("disabling an enabled mechanic remains valid");
    assert_eq!(error.value, json!(1));
}

#[test]
fn restricted_and_surface_profiles_keep_disabled_mechanics_disabled() {
    for profile in [ProfileName::Restricted, ProfileName::Surface] {
        for (feature, request) in [
            (
                "loops",
                RequestedFeatures {
                    allow_loops: Some(true),
                    ..RequestedFeatures::default()
                },
            ),
            (
                "loop expressions",
                RequestedFeatures {
                    allow_loop_expressions: Some(true),
                    ..RequestedFeatures::default()
                },
            ),
            (
                "switch expressions",
                RequestedFeatures {
                    allow_switch: Some(true),
                    ..RequestedFeatures::default()
                },
            ),
            (
                "statement expressions",
                RequestedFeatures {
                    allow_statement_expressions: Some(true),
                    ..RequestedFeatures::default()
                },
            ),
            (
                "anonymous functions",
                RequestedFeatures {
                    allow_anonymous_functions: Some(true),
                    ..RequestedFeatures::default()
                },
            ),
            (
                "shadowing",
                RequestedFeatures {
                    allow_shadowing: Some(true),
                    ..RequestedFeatures::default()
                },
            ),
        ] {
            let error = evaluate(EvaluateScriptArgs {
                script: "1".to_string(),
                profile: Some(profile),
                features: Some(request),
                ..EvaluateScriptArgs::default()
            })
            .expect_err("a disabled mechanic must not be re-enabled");
            assert_eq!(
                error.code.as_str(),
                "feature_widened",
                "{profile:?} unexpectedly widened {feature}"
            );
        }
    }

    let error = evaluate(EvaluateScriptArgs {
        script: "let f = |value| value + 1; f(2)".to_string(),
        profile: Some(ProfileName::Restricted),
        ..EvaluateScriptArgs::default()
    })
    .expect_err("restricted profile must reject closure syntax");
    assert_eq!(error.code.as_str(), "parse_error");

    let error = evaluate(EvaluateScriptArgs {
        script: "let f = |value| value + 1; f(2)".to_string(),
        profile: Some(ProfileName::Surface),
        ..EvaluateScriptArgs::default()
    })
    .expect_err("surface profile must reject closure syntax");
    assert_eq!(error.code.as_str(), "parse_error");
}

#[test]
fn runaway_loop_hits_operation_limit() {
    let error =
        run("let n = 0; while true { n += 1; }").expect_err("unbounded loops must be stopped");
    assert_eq!(error.code.as_str(), "operation_limit");
}

#[test]
fn cooperative_cancellation_stops_before_operation_ceiling() {
    let error = evaluate(EvaluateScriptArgs {
        script: "while true { }".to_string(),
        cancel_after_operations: Some(10),
        ..EvaluateScriptArgs::default()
    })
    .expect_err("cancellation threshold should terminate evaluation");
    assert_eq!(error.code.as_str(), "cancelled");

    let error = evaluate(EvaluateScriptArgs {
        script: "40 + 2".to_string(),
        cancelled: true,
        ..EvaluateScriptArgs::default()
    })
    .expect_err("pre-cancelled requests do not execute");
    assert_eq!(error.code.as_str(), "cancelled");
}

#[test]
fn recursive_script_hits_call_depth_limit() {
    let error = run("fn recurse() { recurse(); } recurse();")
        .expect_err("recursive scripts must hit the call-depth ceiling");
    assert_eq!(error.code.as_str(), "call_depth_limit");
}

#[test]
fn collection_and_output_limits_are_enforced() {
    let error = run("let a = []; for x in 0..200 { a.push(x); } a")
        .expect_err("arrays above the profile ceiling must be rejected");
    assert_eq!(error.code.as_str(), "data_limit");

    let error = run("for x in 0..20000 { print(\"xxxxxxxxxxxxxxxx\"); }")
        .expect_err("captured output above the profile ceiling must be rejected");
    assert_eq!(error.code.as_str(), "output_limit");
}

#[test]
fn aggregate_output_limit_covers_result_and_printed_output() {
    let error = evaluate(EvaluateScriptArgs {
        script: r#"print("123456789012345678901234567890123456"); "abcdefghijklmnopqrstuvwxyz123456789""#
            .to_string(),
        limits: Some(RequestedLimits {
            max_output_bytes: Some(64),
            ..RequestedLimits::default()
        }),
        ..EvaluateScriptArgs::default()
    })
    .expect_err("result plus captured output must share one aggregate bound");
    assert_eq!(error.code.as_str(), "output_limit");
}

#[test]
fn host_paths_and_sleep_are_unavailable() {
    let error = run(r#"import "fs" as fs; fs::read_file("home://secret")"#)
        .expect_err("module imports must not have a host resolver");
    assert_eq!(error.code.as_str(), "capability_denied");

    let error = run("sleep(1)").expect_err("sleep must not block the capsule");
    assert_eq!(error.code.as_str(), "unsupported_function");

    for script in ["random()", "rand()"] {
        let error = run(script).expect_err("randomness must not be script-visible");
        assert_eq!(error.code.as_str(), "unsupported_function", "{script}");
    }
}

#[test]
fn dynamic_eval_is_disabled_before_nested_source_execution() {
    for profile in ProfileName::ALL {
        for script in ["eval(\"1 + 1\")", "eval(input[\"code\"])"] {
            let error = evaluate(EvaluateScriptArgs {
                script: script.to_string(),
                input: json!({"code": "while true { print(\"nested\"); }"}),
                profile: Some(profile),
                cancel_after_operations: Some(1),
                ..EvaluateScriptArgs::default()
            })
            .expect_err("nested source must never execute");
            assert_eq!(error.code.as_str(), "dynamic_eval_disabled");
            assert_eq!(error.to_string(), "rhai:dynamic_eval_disabled");
        }
    }

    let error = run(r#"let pointer = Fn("eval"); call(pointer, "1 + 1")"#)
        .expect_err("function-pointer reflection must not recover eval");
    assert_eq!(error.code.as_str(), "parse_error");
    assert_eq!(error.to_string(), "rhai:parse_error");
}

#[test]
fn unknown_tool_fields_are_rejected_before_evaluation() {
    let error = serde_json::from_str::<EvaluateScriptArgs>(
        r#"{"script":"1 + 1","unknown_tool_field":true}"#,
    )
    .expect_err("unknown tool fields must fail closed");
    assert!(error.to_string().contains("unknown field"));

    let error = serde_json::from_str::<EvaluateScriptArgs>(
        r#"{"script":"1 + 1","limits":{"unknown_limit":1}}"#,
    )
    .expect_err("unknown nested limit fields must fail closed");
    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn empty_args_reject_unknown_fields() {
    let error = serde_json::from_str::<EmptyArgs>(r#"{"future_field":true}"#)
        .expect_err("list_script_profiles must reject unknown fields");
    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn each_evaluation_has_fresh_functions_and_scope() {
    let first = run("fn private_value() { 41 } private_value()")
        .expect("first invocation should define its own function");
    assert_eq!(first.value, json!(41));

    let error = run("private_value()")
        .expect_err("script functions must not leak into the next invocation");
    assert_eq!(error.code.as_str(), "unsupported_function");
}

#[test]
fn source_and_input_sizes_are_preflighted() {
    let error = evaluate(EvaluateScriptArgs {
        script: "x".repeat(1024),
        limits: Some(RequestedLimits {
            max_script_bytes: Some(8),
            ..RequestedLimits::default()
        }),
        ..EvaluateScriptArgs::default()
    })
    .expect_err("source size must be checked before parsing");
    assert_eq!(error.code.as_str(), "script_too_large");

    let error = evaluate(EvaluateScriptArgs {
        script: "input".to_string(),
        input: json!("0123456789"),
        limits: Some(RequestedLimits {
            max_input_bytes: Some(4),
            ..RequestedLimits::default()
        }),
        ..EvaluateScriptArgs::default()
    })
    .expect_err("input size must be checked before conversion");
    assert_eq!(error.code.as_str(), "input_too_large");
}
