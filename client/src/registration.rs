use super::http;
use aio_plugin_identity_model::RegisterRequest;
use az_ui_components::{
    admin::StatusMessage,
    button::{Button, ButtonVariant},
    dialog::{Dialog, DialogTitle},
    input::Input,
};
use dioxus::prelude::*;

#[component]
pub(super) fn RegistrationDialog(on_close: Callback<()>) -> Element {
    let mut account = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut confirmation = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    rsx! {
        Dialog { open: true, on_open_change: move |open: bool| if !open && !busy() { on_close.call(()) },
            DialogTitle { "注册账号" }
            form { class: "admin-form", onsubmit: move |event: FormEvent| {
                event.prevent_default();
                if busy() { return; }
                if password() != confirmation() { error.set(Some("两次输入的密码不一致".into())); return; }
                busy.set(true); error.set(None);
                let request = RegisterRequest { account: account(), password: password() };
                spawn(async move {
                    match http::register_account(request).await {
                        Ok(()) => on_close.call(()),
                        Err(message) => error.set(Some(message)),
                    }
                    busy.set(false);
                });
            },
                fieldset { class: "admin-form", disabled: busy(),
                    label { class: "admin-field", span { "账号" }
                        Input { name: "account", aria_label: "账号", autocomplete: "username", required: true, maxlength: 64,
                            value: account(), oninput: move |e: FormEvent| account.set(e.value()) }
                    }
                    label { class: "admin-field", span { "密码" }
                        Input { name: "password", aria_label: "密码", r#type: "password", autocomplete: "new-password", required: true, maxlength: 128,
                            value: password(), oninput: move |e: FormEvent| password.set(e.value()) }
                    }
                    label { class: "admin-field", span { "确认密码" }
                        Input { name: "confirmation", aria_label: "确认密码", r#type: "password", autocomplete: "new-password", required: true, maxlength: 128,
                            value: confirmation(), oninput: move |e: FormEvent| confirmation.set(e.value()) }
                    }
                }
                if let Some(message) = error() { StatusMessage { error: true, message } }
                footer { class: "admin-actions",
                    Button { r#type: "button", variant: ButtonVariant::Ghost, disabled: busy(), onclick: move |_| on_close.call(()), "取消" }
                    Button { r#type: "submit", disabled: busy(), if busy() { "正在注册" } else { "注册并登录" } }
                }
            }
        }
    }
}
