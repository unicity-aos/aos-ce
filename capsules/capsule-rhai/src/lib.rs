#![deny(unsafe_code)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![allow(missing_docs)]

//! A bounded Rhai capsule for user-space recipes.
//!
//! The capsule imports Astrid SDK plumbing for its tool entry points, but the
//! script language receives no filesystem, network, process, clock, IPC, or
//! credential primitives.  It turns a per-invocation JSON request into a fresh
//! Rhai engine, applies an immutable set of ceilings, and returns a JSON value
//! plus captured output.  Requests may select a named profile and narrow its
//! limits, but can never widen a profile or the hard capsule ceiling.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use astrid_sdk::prelude::*;
use astrid_sdk::schemars;
use rhai::packages::{
    ArithmeticPackage, BasicArrayPackage, BasicBlobPackage, BasicFnPackage, BasicIteratorPackage,
    BasicMapPackage, BasicMathPackage, BasicStringPackage, BitFieldPackage, LogicPackage,
    MoreStringPackage, Package,
};
use rhai::{Dynamic, Engine, EvalAltResult, LexError, ParseErrorType, Scope};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// These values are part of the component's immutable policy.  A caller can
// request lower values in a request, but no profile or request can raise them.
const HARD_MAX_SCRIPT_BYTES: u64 = 64 * 1024;
const HARD_MAX_INPUT_BYTES: u64 = 64 * 1024;
const HARD_MAX_OUTPUT_BYTES: u64 = 16 * 1024;
const HARD_MAX_OPERATIONS: u64 = 100_000;
const HARD_MAX_CALL_LEVELS: u64 = 64;
const HARD_MAX_EXPR_DEPTH: u64 = 64;
const HARD_MAX_FUNCTION_EXPR_DEPTH: u64 = 32;
const HARD_MAX_VARIABLES: u64 = 128;
const HARD_MAX_FUNCTIONS: u64 = 32;
const HARD_MAX_STRING_BYTES: u64 = 16 * 1024;
const HARD_MAX_ARRAY_ITEMS: u64 = 256;
const HARD_MAX_MAP_ENTRIES: u64 = 128;

const CANCELLED_TOKEN: &str = "cancelled";
const OUTPUT_LIMIT_TOKEN: &str = "output_limit";

/// A named, immutable baseline profile.
#[derive(
    Debug, Clone, Copy, Default, Deserialize, Serialize, schemars::JsonSchema, PartialEq, Eq,
)]
#[serde(rename_all = "snake_case")]
pub enum ProfileName {
    /// General-purpose bounded recipes.
    #[default]
    Default,
    /// Small, expression-oriented scripts with no loops or functions.
    Restricted,
    /// Surface behaviour profile: no loops/functions and a small data budget.
    Surface,
}

impl ProfileName {
    const ALL: [Self; 3] = [Self::Default, Self::Restricted, Self::Surface];

    /// Return the stable wire identifier for this profile.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Restricted => "restricted",
            Self::Surface => "surface",
        }
    }
}

/// Language mechanics that may be disabled by a profile or narrowed request.
#[derive(Debug, Clone, Copy, Serialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct Features {
    /// Permit `while`, `for`, and `loop` statements.
    pub allow_loops: bool,
    /// Permit loop expressions such as `loop { ... }`.
    pub allow_loop_expressions: bool,
    /// Permit `if` expressions.
    pub allow_if: bool,
    /// Permit `switch` expressions.
    pub allow_switch: bool,
    /// Permit statement-valued expressions.
    pub allow_statement_expressions: bool,
    /// Permit anonymous function expressions.
    pub allow_anonymous_functions: bool,
    /// Permit a nested scope to shadow a variable.
    pub allow_shadowing: bool,
    /// Require variables to be declared before use.
    pub strict_variables: bool,
}

/// Resource ceilings applied to one invocation.
#[derive(Debug, Clone, Copy, Serialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct Limits {
    /// Maximum Rhai operations.
    pub max_operations: u64,
    /// Maximum nested function-call levels.
    pub max_call_levels: u64,
    /// Maximum expression nesting depth.
    pub max_expr_depth: u64,
    /// Maximum expression depth within a function.
    pub max_function_expr_depth: u64,
    /// Maximum variables in a scope.
    pub max_variables: u64,
    /// Maximum scripted functions in the program.
    pub max_functions: u64,
    /// Maximum string length in bytes.
    pub max_string_bytes: u64,
    /// Maximum array elements.
    pub max_array_items: u64,
    /// Maximum object-map entries.
    pub max_map_entries: u64,
    /// Maximum script source size in bytes.
    pub max_script_bytes: u64,
    /// Maximum serialized input size in bytes.
    pub max_input_bytes: u64,
    /// Maximum combined serialized result and captured output in bytes.
    pub max_output_bytes: u64,
}

/// A profile and its feature/limit defaults returned by `list_script_profiles`.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct ProfileDescription {
    /// Stable profile identifier.
    pub name: ProfileName,
    /// Human-readable purpose of the profile.
    pub description: &'static str,
    /// Language mechanics enabled by default.
    pub features: Features,
    /// Immutable profile ceilings.
    pub limits: Limits,
}

/// Optional stricter numeric limits for one request.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RequestedLimits {
    /// Maximum Rhai operations; must not exceed the selected profile.
    pub max_operations: Option<u64>,
    /// Maximum nested function-call levels.
    pub max_call_levels: Option<u64>,
    /// Maximum expression nesting depth.
    pub max_expr_depth: Option<u64>,
    /// Maximum expression depth within a function.
    pub max_function_expr_depth: Option<u64>,
    /// Maximum variables in a scope.
    pub max_variables: Option<u64>,
    /// Maximum scripted functions in the program.
    pub max_functions: Option<u64>,
    /// Maximum string length in bytes.
    pub max_string_bytes: Option<u64>,
    /// Maximum array elements.
    pub max_array_items: Option<u64>,
    /// Maximum object-map entries.
    pub max_map_entries: Option<u64>,
    /// Maximum script source size in bytes.
    pub max_script_bytes: Option<u64>,
    /// Maximum serialized input size in bytes.
    pub max_input_bytes: Option<u64>,
    /// Maximum combined serialized result and captured output in bytes.
    pub max_output_bytes: Option<u64>,
}

/// Optional language-mechanic restrictions for one request.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RequestedFeatures {
    /// Disable loops by setting this to `false`.
    pub allow_loops: Option<bool>,
    /// Disable loop expressions by setting this to `false`.
    pub allow_loop_expressions: Option<bool>,
    /// Disable `if` expressions by setting this to `false`.
    pub allow_if: Option<bool>,
    /// Disable `switch` expressions by setting this to `false`.
    pub allow_switch: Option<bool>,
    /// Disable statement expressions by setting this to `false`.
    pub allow_statement_expressions: Option<bool>,
    /// Disable anonymous functions by setting this to `false`.
    pub allow_anonymous_functions: Option<bool>,
    /// Disable variable shadowing by setting this to `false`.
    pub allow_shadowing: Option<bool>,
    /// Enable strict variables only when the selected profile already enables it.
    pub strict_variables: Option<bool>,
}

/// Arguments to the `evaluate_script` tool.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct EvaluateScriptArgs {
    /// Rhai source to evaluate.
    pub script: String,
    /// JSON value exposed to the script as the `input` variable.
    pub input: Value,
    /// Named immutable baseline profile.  Defaults to `default`.
    pub profile: Option<ProfileName>,
    /// Optional per-invocation limits that may only narrow the profile.
    pub limits: Option<RequestedLimits>,
    /// Optional per-invocation language restrictions that may only disable mechanics.
    pub features: Option<RequestedFeatures>,
    /// Reject this invocation before parsing when already cancelled.
    pub cancelled: bool,
    /// Cooperative cancellation threshold in Rhai operations.
    pub cancel_after_operations: Option<u64>,
}

/// Successful result from `evaluate_script`.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct EvaluateScriptResponse {
    /// Language-neutral script-runtime contract identifier.
    pub contract: &'static str,
    /// Runtime implementation identifier.
    pub runtime: &'static str,
    /// Effective named profile.
    pub profile: ProfileName,
    /// Script return value converted back to JSON.
    pub value: Value,
    /// Text emitted by `print`/`debug`, bounded by `max_output_bytes`.
    pub output: String,
    /// Number of Rhai operations observed by the progress callback.
    pub operations: u64,
    /// Effective limits after request narrowing.
    pub limits: Limits,
}

#[derive(Debug, Clone, Copy)]
struct RuntimeConfig {
    profile: ProfileName,
    features: Features,
    limits: Limits,
}

#[derive(Debug, Clone, Copy)]
enum FailureCode {
    InvalidRequest,
    ScriptTooLarge,
    InputTooLarge,
    LimitWidened,
    FeatureWidened,
    OperationLimit,
    CallDepthLimit,
    ExpressionDepthLimit,
    FunctionLimit,
    VariableLimit,
    DataLimit,
    OutputLimit,
    Cancelled,
    ParseError,
    DynamicEvalDisabled,
    UnsupportedFunction,
    CapabilityDenied,
    RuntimeError,
    SerializationError,
    InternalError,
}

impl FailureCode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::ScriptTooLarge => "script_too_large",
            Self::InputTooLarge => "input_too_large",
            Self::LimitWidened => "limit_widened",
            Self::FeatureWidened => "feature_widened",
            Self::OperationLimit => "operation_limit",
            Self::CallDepthLimit => "call_depth_limit",
            Self::ExpressionDepthLimit => "expression_depth_limit",
            Self::FunctionLimit => "function_limit",
            Self::VariableLimit => "variable_limit",
            Self::DataLimit => "data_limit",
            Self::OutputLimit => "output_limit",
            Self::Cancelled => "cancelled",
            Self::ParseError => "parse_error",
            Self::DynamicEvalDisabled => "dynamic_eval_disabled",
            Self::UnsupportedFunction => "unsupported_function",
            Self::CapabilityDenied => "capability_denied",
            Self::RuntimeError => "runtime_error",
            Self::SerializationError => "serialization_error",
            Self::InternalError => "internal_error",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct EvalFailure {
    code: FailureCode,
}

impl EvalFailure {
    const fn new(code: FailureCode) -> Self {
        Self { code }
    }
}

impl fmt::Display for EvalFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "rhai:{}", self.code.as_str())
    }
}

struct ExecutionState {
    operations: AtomicU64,
    cancelled: AtomicBool,
    output_exceeded: AtomicBool,
    output: Mutex<String>,
    max_output_bytes: usize,
}

impl ExecutionState {
    fn new(max_output_bytes: u64) -> Result<Arc<Self>, EvalFailure> {
        let max_output_bytes = usize::try_from(max_output_bytes)
            .map_err(|_| EvalFailure::new(FailureCode::InvalidRequest))?;
        Ok(Arc::new(Self {
            operations: AtomicU64::new(0),
            cancelled: AtomicBool::new(false),
            output_exceeded: AtomicBool::new(false),
            output: Mutex::new(String::new()),
            max_output_bytes,
        }))
    }

    fn record_output(&self, text: &str) {
        let Ok(mut output) = self.output.lock() else {
            self.output_exceeded.store(true, Ordering::Relaxed);
            return;
        };

        let available = self.max_output_bytes.saturating_sub(output.len());
        if available == 0 {
            self.output_exceeded.store(true, Ordering::Relaxed);
            return;
        }

        let mut end = available.min(text.len());
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        output.push_str(&text[..end]);
        if end < text.len() {
            self.output_exceeded.store(true, Ordering::Relaxed);
        }
    }

    fn captured_output(&self) -> Result<String, EvalFailure> {
        self.output
            .lock()
            .map(|output| output.clone())
            .map_err(|_| EvalFailure::new(FailureCode::InternalError))
    }
}

fn profile_defaults(profile: ProfileName) -> RuntimeConfig {
    let (features, limits) = match profile {
        ProfileName::Default => (
            Features {
                allow_loops: true,
                allow_loop_expressions: true,
                allow_if: true,
                allow_switch: true,
                allow_statement_expressions: true,
                allow_anonymous_functions: true,
                allow_shadowing: true,
                strict_variables: false,
            },
            Limits {
                max_operations: 50_000,
                max_call_levels: 32,
                max_expr_depth: 48,
                max_function_expr_depth: 24,
                max_variables: 64,
                max_functions: 32,
                max_string_bytes: 8 * 1024,
                max_array_items: 128,
                max_map_entries: 64,
                max_script_bytes: 32 * 1024,
                max_input_bytes: 32 * 1024,
                max_output_bytes: 16 * 1024,
            },
        ),
        ProfileName::Restricted => (
            Features {
                allow_loops: false,
                allow_loop_expressions: false,
                allow_if: true,
                allow_switch: false,
                allow_statement_expressions: false,
                allow_anonymous_functions: false,
                allow_shadowing: false,
                strict_variables: true,
            },
            Limits {
                max_operations: 8_000,
                max_call_levels: 8,
                max_expr_depth: 16,
                max_function_expr_depth: 8,
                max_variables: 16,
                max_functions: 0,
                max_string_bytes: 4 * 1024,
                max_array_items: 32,
                max_map_entries: 16,
                max_script_bytes: 16 * 1024,
                max_input_bytes: 16 * 1024,
                max_output_bytes: 4 * 1024,
            },
        ),
        ProfileName::Surface => (
            Features {
                allow_loops: false,
                allow_loop_expressions: false,
                allow_if: true,
                allow_switch: false,
                allow_statement_expressions: false,
                allow_anonymous_functions: false,
                allow_shadowing: false,
                strict_variables: true,
            },
            Limits {
                max_operations: 20_000,
                max_call_levels: 8,
                max_expr_depth: 24,
                max_function_expr_depth: 8,
                max_variables: 32,
                max_functions: 0,
                max_string_bytes: 8 * 1024,
                max_array_items: 64,
                max_map_entries: 32,
                max_script_bytes: 16 * 1024,
                max_input_bytes: 32 * 1024,
                max_output_bytes: 8 * 1024,
            },
        ),
    };

    RuntimeConfig {
        profile,
        features,
        limits: limit_to_hard_ceiling(limits),
    }
}

/// Keep every named profile below the immutable component ceiling.  Keeping
/// this clamp next to the profile table makes the invariant explicit if a
/// future profile is edited without updating the constants above.
fn limit_to_hard_ceiling(limits: Limits) -> Limits {
    Limits {
        max_operations: limits.max_operations.min(HARD_MAX_OPERATIONS),
        max_call_levels: limits.max_call_levels.min(HARD_MAX_CALL_LEVELS),
        max_expr_depth: limits.max_expr_depth.min(HARD_MAX_EXPR_DEPTH),
        max_function_expr_depth: limits
            .max_function_expr_depth
            .min(HARD_MAX_FUNCTION_EXPR_DEPTH),
        max_variables: limits.max_variables.min(HARD_MAX_VARIABLES),
        max_functions: limits.max_functions.min(HARD_MAX_FUNCTIONS),
        max_string_bytes: limits.max_string_bytes.min(HARD_MAX_STRING_BYTES),
        max_array_items: limits.max_array_items.min(HARD_MAX_ARRAY_ITEMS),
        max_map_entries: limits.max_map_entries.min(HARD_MAX_MAP_ENTRIES),
        max_script_bytes: limits.max_script_bytes.min(HARD_MAX_SCRIPT_BYTES),
        max_input_bytes: limits.max_input_bytes.min(HARD_MAX_INPUT_BYTES),
        max_output_bytes: limits.max_output_bytes.min(HARD_MAX_OUTPUT_BYTES),
    }
}

fn profile_description(profile: ProfileName) -> ProfileDescription {
    let config = profile_defaults(profile);
    let description = match profile {
        ProfileName::Default => "Bounded general-purpose recipe execution",
        ProfileName::Restricted => "Small expression-oriented scripts without loops or functions",
        ProfileName::Surface => "Deterministic surface behaviour without loops or functions",
    };
    ProfileDescription {
        name: profile,
        description,
        features: config.features,
        limits: config.limits,
    }
}

fn narrow_limit(requested: Option<u64>, baseline: u64) -> Result<u64, EvalFailure> {
    match requested {
        None => Ok(baseline),
        Some(0) => Err(EvalFailure::new(FailureCode::InvalidRequest)),
        Some(value) if value > baseline => Err(EvalFailure::new(FailureCode::LimitWidened)),
        Some(value) => Ok(value),
    }
}

fn narrow_feature(requested: Option<bool>, baseline: bool) -> Result<bool, EvalFailure> {
    match requested {
        None => Ok(baseline),
        Some(true) if !baseline => Err(EvalFailure::new(FailureCode::FeatureWidened)),
        Some(value) => Ok(baseline && value),
    }
}

fn effective_config(
    args: &EvaluateScriptArgs,
) -> Result<(RuntimeConfig, Option<u64>), EvalFailure> {
    let profile = args.profile.unwrap_or_default();
    let mut config = profile_defaults(profile);
    if let Some(requested) = &args.limits {
        config.limits = Limits {
            max_operations: narrow_limit(requested.max_operations, config.limits.max_operations)?,
            max_call_levels: narrow_limit(
                requested.max_call_levels,
                config.limits.max_call_levels,
            )?,
            max_expr_depth: narrow_limit(requested.max_expr_depth, config.limits.max_expr_depth)?,
            max_function_expr_depth: narrow_limit(
                requested.max_function_expr_depth,
                config.limits.max_function_expr_depth,
            )?,
            max_variables: narrow_limit(requested.max_variables, config.limits.max_variables)?,
            max_functions: narrow_limit(requested.max_functions, config.limits.max_functions)?,
            max_string_bytes: narrow_limit(
                requested.max_string_bytes,
                config.limits.max_string_bytes,
            )?,
            max_array_items: narrow_limit(
                requested.max_array_items,
                config.limits.max_array_items,
            )?,
            max_map_entries: narrow_limit(
                requested.max_map_entries,
                config.limits.max_map_entries,
            )?,
            max_script_bytes: narrow_limit(
                requested.max_script_bytes,
                config.limits.max_script_bytes,
            )?,
            max_input_bytes: narrow_limit(
                requested.max_input_bytes,
                config.limits.max_input_bytes,
            )?,
            max_output_bytes: narrow_limit(
                requested.max_output_bytes,
                config.limits.max_output_bytes,
            )?,
        };
    }

    if let Some(requested) = &args.features {
        config.features = Features {
            allow_loops: narrow_feature(requested.allow_loops, config.features.allow_loops)?,
            allow_loop_expressions: narrow_feature(
                requested.allow_loop_expressions,
                config.features.allow_loop_expressions,
            )?,
            allow_if: narrow_feature(requested.allow_if, config.features.allow_if)?,
            allow_switch: narrow_feature(requested.allow_switch, config.features.allow_switch)?,
            allow_statement_expressions: narrow_feature(
                requested.allow_statement_expressions,
                config.features.allow_statement_expressions,
            )?,
            allow_anonymous_functions: narrow_feature(
                requested.allow_anonymous_functions,
                config.features.allow_anonymous_functions,
            )?,
            allow_shadowing: narrow_feature(
                requested.allow_shadowing,
                config.features.allow_shadowing,
            )?,
            strict_variables: narrow_feature(
                requested.strict_variables,
                config.features.strict_variables,
            )?,
        };
    }

    let cancel_after_operations = match args.cancel_after_operations {
        None => None,
        Some(0) => return Err(EvalFailure::new(FailureCode::InvalidRequest)),
        Some(value) if value > config.limits.max_operations => {
            return Err(EvalFailure::new(FailureCode::LimitWidened));
        }
        Some(value) => Some(value),
    };

    Ok((config, cancel_after_operations))
}

fn to_usize(value: u64) -> Result<usize, EvalFailure> {
    usize::try_from(value).map_err(|_| EvalFailure::new(FailureCode::InvalidRequest))
}

fn build_engine(
    config: RuntimeConfig,
    state: &Arc<ExecutionState>,
    cancel_after_operations: Option<u64>,
) -> Result<Engine, EvalFailure> {
    let mut engine = Engine::new_raw();

    // Register only deterministic, in-memory packages.  In particular, do
    // not register StandardPackage: it also exposes the clock package.  The
    // LanguageCorePackage bundled in CorePackage contains a blocking `sleep`
    // function, so register the pure core packages individually instead of
    // trying to disable that function after registration.
    ArithmeticPackage::new().register_into_engine(&mut engine);
    BasicStringPackage::new().register_into_engine(&mut engine);
    BasicIteratorPackage::new().register_into_engine(&mut engine);
    BasicFnPackage::new().register_into_engine(&mut engine);
    BitFieldPackage::new().register_into_engine(&mut engine);
    LogicPackage::new().register_into_engine(&mut engine);
    BasicMathPackage::new().register_into_engine(&mut engine);
    BasicArrayPackage::new().register_into_engine(&mut engine);
    BasicBlobPackage::new().register_into_engine(&mut engine);
    BasicMapPackage::new().register_into_engine(&mut engine);
    MoreStringPackage::new().register_into_engine(&mut engine);

    engine
        .set_optimization_level(rhai::OptimizationLevel::None)
        .set_max_operations(config.limits.max_operations)
        .set_max_call_levels(to_usize(config.limits.max_call_levels)?)
        .set_max_expr_depths(
            to_usize(config.limits.max_expr_depth)?,
            to_usize(config.limits.max_function_expr_depth)?,
        )
        .set_max_variables(to_usize(config.limits.max_variables)?)
        .set_max_functions(to_usize(config.limits.max_functions)?)
        .set_max_string_size(to_usize(config.limits.max_string_bytes)?)
        .set_max_array_size(to_usize(config.limits.max_array_items)?)
        .set_max_map_size(to_usize(config.limits.max_map_entries)?)
        .set_allow_looping(config.features.allow_loops)
        .set_allow_loop_expressions(config.features.allow_loop_expressions)
        .set_allow_if_expression(config.features.allow_if)
        .set_allow_switch_expression(config.features.allow_switch)
        .set_allow_statement_expression(config.features.allow_statement_expressions)
        .set_allow_anonymous_fn(config.features.allow_anonymous_functions)
        .set_allow_shadowing(config.features.allow_shadowing)
        .set_strict_variables(config.features.strict_variables);

    // Rhai's `eval` keyword executes a second source string in the current
    // scope.  Disable it in the tokenizer, before parsing, so nested source
    // cannot bypass this invocation's source-size, feature, or profile
    // preflight.  This is stronger than merely omitting a registered function:
    // `eval` is a language keyword handled specially by the VM.
    engine.disable_symbol("eval");

    let output_state = Arc::clone(state);
    engine.on_print(move |text| output_state.record_output(text));

    let debug_state = Arc::clone(state);
    engine.on_debug(move |text, _source, _position| debug_state.record_output(text));

    let progress_state = Arc::clone(state);
    engine.on_progress(move |operations| {
        progress_state
            .operations
            .store(operations, Ordering::Relaxed);
        if progress_state.output_exceeded.load(Ordering::Relaxed) {
            return Some(Dynamic::from(OUTPUT_LIMIT_TOKEN));
        }
        if let Some(threshold) = cancel_after_operations
            && operations >= threshold
        {
            progress_state.cancelled.store(true, Ordering::Relaxed);
            return Some(Dynamic::from(CANCELLED_TOKEN));
        }
        None
    });

    Ok(engine)
}

fn parse_failure(error: &ParseErrorType) -> FailureCode {
    match error {
        ParseErrorType::BadInput(LexError::ImproperSymbol(symbol, _)) if symbol == "eval" => {
            FailureCode::DynamicEvalDisabled
        }
        ParseErrorType::Reserved(symbol) if symbol == "import" => FailureCode::CapabilityDenied,
        ParseErrorType::TooManyFunctions => FailureCode::FunctionLimit,
        ParseErrorType::ExprTooDeep => FailureCode::ExpressionDepthLimit,
        ParseErrorType::LiteralTooLarge(_, _) => FailureCode::DataLimit,
        ParseErrorType::VariableExists(_) | ParseErrorType::VariableUndefined(_) => {
            FailureCode::RuntimeError
        }
        _ => FailureCode::ParseError,
    }
}

fn classify_eval_error(error: &EvalAltResult, state: &ExecutionState) -> FailureCode {
    if state.output_exceeded.load(Ordering::Relaxed) {
        return FailureCode::OutputLimit;
    }
    if state.cancelled.load(Ordering::Relaxed) {
        return FailureCode::Cancelled;
    }

    match error {
        EvalAltResult::ErrorParsing(error, _) => parse_failure(error),
        EvalAltResult::ErrorTooManyOperations(_) => FailureCode::OperationLimit,
        EvalAltResult::ErrorTooManyVariables(_) => FailureCode::VariableLimit,
        EvalAltResult::ErrorTooManyModules(_) | EvalAltResult::ErrorModuleNotFound(_, _) => {
            FailureCode::CapabilityDenied
        }
        EvalAltResult::ErrorStackOverflow(_) => FailureCode::CallDepthLimit,
        EvalAltResult::ErrorDataTooLarge(_, _) => FailureCode::DataLimit,
        EvalAltResult::ErrorTerminated(_, _) => FailureCode::Cancelled,
        EvalAltResult::ErrorFunctionNotFound(_, _) => FailureCode::UnsupportedFunction,
        EvalAltResult::ErrorInModule(_, inner, _)
        | EvalAltResult::ErrorInFunctionCall(_, _, inner, _) => classify_eval_error(inner, state),
        EvalAltResult::ErrorSystem(_, _) => FailureCode::RuntimeError,
        _ => FailureCode::RuntimeError,
    }
}

fn input_size(input: &Value) -> Result<usize, EvalFailure> {
    serde_json::to_vec(input)
        .map(|bytes| bytes.len())
        .map_err(|_| EvalFailure::new(FailureCode::SerializationError))
}

fn evaluate(args: EvaluateScriptArgs) -> Result<EvaluateScriptResponse, EvalFailure> {
    let (config, cancel_after_operations) = effective_config(&args)?;

    if args.cancelled {
        return Err(EvalFailure::new(FailureCode::Cancelled));
    }
    if args.script.as_bytes().contains(&0) {
        return Err(EvalFailure::new(FailureCode::InvalidRequest));
    }
    if args.script.len() as u64 > config.limits.max_script_bytes {
        return Err(EvalFailure::new(FailureCode::ScriptTooLarge));
    }
    if input_size(&args.input)? as u64 > config.limits.max_input_bytes {
        return Err(EvalFailure::new(FailureCode::InputTooLarge));
    }

    let state = ExecutionState::new(config.limits.max_output_bytes)?;
    let engine = build_engine(config, &state, cancel_after_operations)?;
    let input = rhai::serde::to_dynamic(&args.input)
        .map_err(|_| EvalFailure::new(FailureCode::SerializationError))?;
    engine
        .ensure_data_size_within_limits(&input)
        .map_err(|_| EvalFailure::new(FailureCode::DataLimit))?;

    let mut scope = Scope::new();
    scope.push_dynamic("input", input);
    let value = engine
        .eval_with_scope::<Dynamic>(&mut scope, &args.script)
        .map_err(|error| EvalFailure::new(classify_eval_error(&error, &state)))?;

    if state.output_exceeded.load(Ordering::Relaxed) {
        return Err(EvalFailure::new(FailureCode::OutputLimit));
    }
    if state.cancelled.load(Ordering::Relaxed) {
        return Err(EvalFailure::new(FailureCode::Cancelled));
    }

    let value: Value = rhai::serde::from_dynamic(&value)
        .map_err(|_| EvalFailure::new(FailureCode::SerializationError))?;
    let serialized_value = serde_json::to_vec(&value)
        .map_err(|_| EvalFailure::new(FailureCode::SerializationError))?;
    let output = state.captured_output()?;
    if serialized_value.len() as u64 > config.limits.max_output_bytes
        || output.len().saturating_add(serialized_value.len()) as u64
            > config.limits.max_output_bytes
    {
        return Err(EvalFailure::new(FailureCode::OutputLimit));
    }

    Ok(EvaluateScriptResponse {
        contract: "script-runtime.v1",
        runtime: "rhai",
        profile: config.profile,
        value,
        output,
        operations: state.operations.load(Ordering::Relaxed),
        limits: config.limits,
    })
}

#[derive(Default)]
pub struct RhaiRuntime;

#[capsule]
impl RhaiRuntime {
    /// Evaluate one bounded Rhai script without script-visible host effects.
    #[astrid::tool("evaluate_script")]
    pub fn evaluate_script(
        &self,
        args: EvaluateScriptArgs,
    ) -> Result<EvaluateScriptResponse, SysError> {
        evaluate(args).map_err(|error| SysError::ApiError(error.to_string()))
    }

    /// List the immutable profiles and their effective default ceilings.
    #[astrid::tool("list_script_profiles")]
    pub fn list_script_profiles(
        &self,
        _args: EmptyArgs,
    ) -> Result<Vec<ProfileDescription>, SysError> {
        Ok(ProfileName::ALL
            .into_iter()
            .map(profile_description)
            .collect())
    }
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct EmptyArgs {}
#[cfg(test)]
mod tests;
