use std::fmt::Debug;

#[cfg(feature = "node-api")]
use napi_derive::napi;

use serde::{Deserialize, Serialize};

#[cfg(feature = "node-api")]
use napi::{
  bindgen_prelude::*,
  threadsafe_function::{
    ErrorStrategy, ThreadSafeResultContext, ThreadsafeFunction, ThreadsafeFunctionCallMode,
  },
  JsFunction,
};
#[cfg(feature = "node-api")]
use rspack_error::Result;

use rspack_core::{
  AssetParserDataUrlOption, AssetParserOptions, CompilerOptionsBuilder, ModuleOptions, ModuleRule,
  ParserOptions,
};

use crate::RawOption;

#[cfg(feature = "node-api")]
type JsLoader = ThreadsafeFunction<Vec<u8>, ErrorStrategy::CalleeHandled>;
// type ModuleRuleFunc = ThreadsafeFunction<Vec<u8>, ErrorStrategy::CalleeHandled>;

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
#[cfg(feature = "node-api")]
#[napi(object)]
pub struct RawModuleRule {
  pub test: Option<String>,
  pub resource: Option<String>,
  pub resource_query: Option<String>,
  // Loader experimental
  #[serde(skip_deserializing)]
  pub func__: Option<JsFunction>,
  #[serde(skip_deserializing)]
  pub uses: Option<Vec<JsFunction>>,
  #[napi(
    ts_type = r#""js" | "jsx" | "ts" | "tsx" | "css" | "json" | "asset" | "asset/resource" | "asset/source" | "asset/inline""#
  )]
  pub r#type: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
#[cfg(not(feature = "node-api"))]
pub struct RawModuleRule {
  pub test: Option<String>,
  pub resource: Option<String>,
  pub resource_query: Option<String>,
  // Loader experimental
  #[serde(skip_deserializing)]
  pub func__: Option<()>,
  #[serde(skip_deserializing)]
  pub uses: Option<()>,
  pub r#type: Option<String>,
}

impl Debug for RawModuleRule {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("RawModuleRule")
      .field("test", &self.test)
      // TODO: More specific (Loader stage 2)
      .field("uses", &"js-loaders")
      .finish()
  }
}

#[derive(Debug, Clone, Deserialize)]
#[cfg(not(feature = "node-api"))]
#[serde(rename_all = "camelCase")]
pub struct RawAssetParserDataUrlOption {
  pub max_size: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg(feature = "node-api")]
#[napi(object)]
#[serde(rename_all = "camelCase")]
pub struct RawAssetParserDataUrlOption {
  pub max_size: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg(feature = "node-api")]
#[napi(object)]
#[serde(rename_all = "camelCase")]
pub struct RawAssetParserOptions {
  pub data_url_condition: Option<RawAssetParserDataUrlOption>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg(not(feature = "node-api"))]
#[serde(rename_all = "camelCase")]
pub struct RawAssetParserOptions {
  pub data_url_condition: Option<RawAssetParserDataUrlOption>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg(feature = "node-api")]
#[napi(object)]
#[serde(rename_all = "camelCase")]
pub struct RawParserOptions {
  pub asset: Option<RawAssetParserOptions>,
}

#[derive(Debug, Clone, Deserialize)]
#[cfg(not(feature = "node-api"))]
#[serde(rename_all = "camelCase")]
pub struct RawParserOptions {
  pub asset: Option<RawAssetParserOptions>,
}

#[derive(Default, Debug, Deserialize)]
#[cfg(not(feature = "node-api"))]
#[serde(rename_all = "camelCase")]
pub struct RawModuleOptions {
  pub rules: Vec<RawModuleRule>,
  pub parser: Option<RawParserOptions>,
}

#[derive(Default, Debug, Deserialize)]
#[cfg(feature = "node-api")]
#[serde(rename_all = "camelCase")]
#[napi(object)]
pub struct RawModuleOptions {
  pub rules: Vec<RawModuleRule>,
  pub parser: Option<RawParserOptions>,
}

#[cfg(feature = "node-api")]
pub struct NodeLoaderAdapter {
  pub loader: JsLoader,
}

#[cfg(feature = "node-api")]
impl NodeLoaderAdapter {
  pub fn unref(&mut self, env: &napi::Env) -> anyhow::Result<()> {
    self
      .loader
      .unref(env)
      .map_err(|e| anyhow::format_err!("failed to unref tsfn: {}", e))
  }
}

#[cfg(feature = "node-api")]
impl Debug for NodeLoaderAdapter {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("NodeLoaderAdapter")
      // TODO: More specific (Loader stage 2)
      .field("loaders", &"..")
      .finish()
  }
}

#[cfg(feature = "node-api")]
static LOADER_CALL_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

#[cfg(feature = "node-api")]
static REGISTERED_LOADER_SENDERS: once_cell::sync::Lazy<
  std::sync::Arc<dashmap::DashMap<u32, tokio::sync::oneshot::Sender<LoaderThreadsafeLoaderResult>>>,
> = once_cell::sync::Lazy::new(Default::default);

#[cfg(feature = "node-api")]
#[async_trait::async_trait]
impl rspack_core::Loader for NodeLoaderAdapter {
  fn name(&self) -> &'static str {
    "node-loader-adapter"
  }

  async fn run(
    &self,
    loader_context: &rspack_core::LoaderContext<'_>,
  ) -> Result<Option<rspack_core::LoaderResult>> {
    use std::sync::atomic::Ordering;

    let loader_context = LoaderContext {
      source: loader_context.source.to_owned().into_bytes(),
      resource: loader_context.resource.to_owned(),
      resource_path: loader_context.resource_path.to_owned(),
      resource_fragment: loader_context.resource_fragment.map(|r| r.to_owned()),
      resource_query: loader_context.resource_query.map(|r| r.to_owned()),
    };

    let current_id = LOADER_CALL_ID.fetch_add(1, Ordering::SeqCst);

    let loader_tsfn_context = LoaderThreadsafeContext {
      p: loader_context,
      id: current_id,
    };

    let (tx, rx) = tokio::sync::oneshot::channel::<LoaderThreadsafeLoaderResult>();

    let result = serde_json::to_vec(&loader_tsfn_context).map_err(|err| {
      napi::Error::from_reason(format!("Failed to serialize loader context: {}", err))
    });

    match REGISTERED_LOADER_SENDERS.entry(current_id) {
      dashmap::mapref::entry::Entry::Vacant(v) => {
        v.insert(tx);
      }
      dashmap::mapref::entry::Entry::Occupied(_) => {
        let err = napi::Error::new(
            napi::Status::Unknown,
            format!(
              "Duplicated call id encountered {}, this is not an expected behavior. Please file an issue.",
              current_id,
            ),
          );
        self
          .loader
          .call(Err(err.clone()), ThreadsafeFunctionCallMode::Blocking);
        return Err(anyhow::Error::from(err).into());
      }
    }

    let status = self
      .loader
      .call(result, ThreadsafeFunctionCallMode::Blocking);

    debug_assert_eq!(status, napi::Status::Ok);

    let loader_result = rx.await.map_err(|err| anyhow::Error::from(err))?;

    Ok(
      loader_result.map(|loader_result| rspack_core::LoaderResult {
        content: rspack_core::Content::from(loader_result.content),
      }),
    )
  }

  fn as_any(&self) -> &dyn std::any::Any {
    self
  }

  fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
    self
  }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LoaderContext {
  pub source: Vec<u8>,
  pub resource: String,
  pub resource_path: String,
  pub resource_query: Option<String>,
  pub resource_fragment: Option<String>,
}

#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct LoaderResult {
  pub content: Vec<u8>,
}

type LoaderThreadsafeLoaderContext = LoaderContext;
type LoaderThreadsafeLoaderResult = Option<LoaderResult>;

#[derive(Serialize, Deserialize, Debug)]
struct LoaderThreadsafeResult {
  id: u32,
  // payload
  p: LoaderThreadsafeLoaderResult,
}

#[derive(Serialize, Deserialize, Debug)]
struct LoaderThreadsafeContext {
  id: u32,
  // payload
  p: LoaderThreadsafeLoaderContext,
}

impl RawOption<ModuleRule> for RawModuleRule {
  fn to_compiler_option(self, _options: &CompilerOptionsBuilder) -> anyhow::Result<ModuleRule> {
    // Even this part is using the plural version of loader, it's recommended to use singular version from js side to reduce overhead (This behavior maybe changed later for advanced usage).
    cfg_if::cfg_if! {
      if #[cfg(feature = "node-api")] {
        let loaders = self
        .uses
        .map(|raw_js_loaders| {
          raw_js_loaders
            .into_iter()
            .map(|raw_js_loader| {
              let js_loader: JsLoader = raw_js_loader
                .create_threadsafe_function(
                  0,
                  |ctx| Ok(vec![Buffer::from(ctx.value)]),
                  |ctx: ThreadSafeResultContext<Promise<Buffer>>| {
                    let return_value = ctx.return_value;

                    ctx
                      .env
                      .execute_tokio_future(
                        async move {
                          let return_value = return_value.await?;

                          let result =
                            serde_json::from_slice::<LoaderThreadsafeResult>(return_value.as_ref())?;

                          if let Some((_, sender)) = REGISTERED_LOADER_SENDERS.remove(&result.id) {
                            sender.send(result.p).map_err(|_| {
                              Error::new(napi::Status::GenericFailure, "unable to send".to_owned())
                            })?;
                          } else {
                            return Err(Error::new(
                              napi::Status::GenericFailure,
                              format!("Loader call id {} not found", result.id),
                            ));
                          }

                          Ok(())
                        },
                        |_env, ret| Ok(ret),
                      )
                      .expect("failed to execute tokio future");
                  },
                )
                .unwrap();
              js_loader
            })
            .collect::<Vec<_>>()
        })
        .unwrap_or_default();

        use rspack_core::ModuleType;

        let module_type = self
        .r#type
        .map(|t| match t.as_str() {
          "js" => Ok(ModuleType::Js),
          "jsx" => Ok(ModuleType::Jsx),
          "ts" => Ok(ModuleType::Ts),
          "tsx" => Ok(ModuleType::Tsx),
          "css" => Ok(ModuleType::Css),
          "json" => Ok(ModuleType::Json),
          "asset" => Ok(ModuleType::Asset),
          "asset/source" => Ok(ModuleType::AssetSource),
          "asset/resource" => Ok(ModuleType::AssetResource),
          "asset/inline" => Ok(ModuleType::AssetInline),
          _ => Err(anyhow::format_err!("Unsupported module type: {}", t)),
        })
        .transpose()?;
        let uses = loaders
        .into_iter()
        .map(|loader| Box::new(NodeLoaderAdapter { loader }) as Box<dyn rspack_core::Loader>)
        .collect();
      } else {
        let module_type = Default::default();
        let uses = Default::default();
      }
    };

    // let func = Box::new(
    //   self
    //     .func__
    //     .map(|func| {
    //       let func: Result<ModuleRuleFunc> = func.create_threadsafe_function(
    //         0,
    //         |ctx| Ok(vec![Buffer::from(ctx.value)]),
    //         |ctx: ThreadSafeResultContext<Buffer>| {
    //           dbg!(ctx.return_value.as_ref());
    //           todo!()
    //         },
    //       );
    //       func
    //     })
    //     .transpose()?,
    // );

    // let module_rule_tsfn: &'static Option<ModuleRuleFunc> = Box::leak(func);

    Ok(ModuleRule {
      test: self.test.map(|reg| regex::Regex::new(&reg)).transpose()?,
      resource_query: self
        .resource_query
        .map(|reg| regex::Regex::new(&reg))
        .transpose()?,
      resource: self
        .resource
        .map(|reg| regex::Regex::new(&reg))
        .transpose()?,
      uses,
      module_type,
      // Loader experimental
      func__: None,
    })
  }

  fn fallback_value(_options: &CompilerOptionsBuilder) -> Self {
    RawModuleRule::default()
  }
}

impl RawOption<Option<ModuleOptions>> for RawModuleOptions {
  fn to_compiler_option(
    self,
    options: &CompilerOptionsBuilder,
  ) -> anyhow::Result<Option<ModuleOptions>> {
    // FIXME: temporary implementation
    Ok(Some(ModuleOptions {
      rules: self
        .rules
        .into_iter()
        .map(|rule| {
          rule
            .to_compiler_option(options)
            .map_err(|err| anyhow::format_err!("failed to convert rule: {}", err))
        })
        .collect::<anyhow::Result<Vec<ModuleRule>>>()?,
      parser: self.parser.map(|x| ParserOptions {
        asset: x.asset.map(|y| AssetParserOptions {
          data_url_condition: y.data_url_condition.map(|a| AssetParserDataUrlOption {
            max_size: a.max_size,
          }),
        }),
      }),
    }))
  }

  fn fallback_value(_options: &CompilerOptionsBuilder) -> Self {
    RawModuleOptions {
      rules: vec![],
      parser: None,
    }
  }
}