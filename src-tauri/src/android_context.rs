//! Bridge Tao 0.35's Activity context to the context required by Android Keyring.
//! Call only from Tauri setup, after the main Activity has been registered.

use jni::{objects::GlobalRef, objects::JObject, JavaVM};
use std::sync::OnceLock;

// Keep the Application (not the Activity) alive across Activity recreation.
static APPLICATION_CONTEXT: OnceLock<Result<GlobalRef, String>> = OnceLock::new();

pub fn initialize() -> Result<(), String> {
    APPLICATION_CONTEXT
        .get_or_init(|| {
            let context = tauri::tao::platform::android::prelude::main_android_context()
                .ok_or_else(|| "Android Activity 尚未初始化".to_string())?;
            // SAFETY: Tao owns these JNI handles; setup runs after Activity creation.
            // JObject is borrowed here and never converted into an owned local ref.
            let vm = unsafe { JavaVM::from_raw(context.java_vm.cast()) }
                .map_err(|error| error.to_string())?;
            let mut env = vm.attach_current_thread().map_err(|error| error.to_string())?;
            let activity = unsafe { JObject::from_raw(context.context_jobject.cast()) };
            let application = env
                .call_method(&activity, "getApplicationContext", "()Landroid/content/Context;", &[])
                .and_then(|value| value.l())
                .map_err(|error| error.to_string())?;
            let application = env.new_global_ref(application).map_err(|error| error.to_string())?;
            if application.as_obj().is_null() {
                return Err("Android Application 上下文为空".to_string());
            }
            // SAFETY: Tao 0.35 does not initialize ndk-context. This OnceLock is
            // the sole initializer, before any keyring calls; the GlobalRef is
            // retained for the process lifetime. Revisit when upgrading Tao.
            unsafe {
                ndk_context::initialize_android_context(
                    context.java_vm,
                    application.as_obj().as_raw().cast(),
                );
            }
            Ok(application)
        })
        .as_ref()
        .map(|_| ())
        .map_err(Clone::clone)
}
