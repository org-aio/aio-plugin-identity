use super::http;
use aio_plugin_identity_model::LoginRequest;
use az_ui_components::{
    admin::StatusMessage,
    button::{Button, ButtonSize, ButtonVariant},
    input::Input,
};
use dioxus::prelude::*;
use dioxus_icons::lucide::{Eye, EyeOff};

#[component]
pub fn LoginPage() -> Element {
    let mut account = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut password_visible = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let mut registering = use_signal(|| false);
    rsx! {
        main { class: "admin-auth",
            section { class: "admin-auth-form", h1 { "AIO IDEA" } h2 { "登录你的工作台" }
                form { class: "admin-form", aria_busy: busy(), onsubmit: move |event: FormEvent| {
                    event.prevent_default(); if busy() { return; } busy.set(true); error.set(None);
                    let payload = LoginRequest { account: account(), password: password() };
                    spawn(async move { if let Err(message) = http::login(payload).await { error.set(Some(message)); } busy.set(false); });
                },
                    fieldset { class: "admin-form", disabled: busy(),
                        label { class: "admin-field", span { "账号" } Input { name: "account", autocomplete: "username", aria_label: "账号", required: true, value: account(), oninput: move |event: FormEvent| account.set(event.value()) } }
                        div { class: "admin-field",
                            label { r#for: "login-password", "密码" }
                            div { style: "position: relative; min-width: 0;",
                                Input {
                                    id: "login-password", name: "password",
                                    r#type: if password_visible() { "text" } else { "password" },
                                    style: "padding-right: 44px;",
                                    autocomplete: "current-password", aria_label: "密码", required: true,
                                    value: password(), oninput: move |event: FormEvent| password.set(event.value()),
                                }
                                Button {
                                    r#type: "button", variant: ButtonVariant::Ghost, size: ButtonSize::IconSm,
                                    style: "position: absolute; right: 4px; top: 50%; transform: translateY(-50%);",
                                    title: if password_visible() { "隐藏密码" } else { "显示密码" },
                                    aria_label: if password_visible() { "隐藏密码" } else { "显示密码" },
                                    aria_controls: "login-password", aria_pressed: password_visible().to_string(),
                                    onclick: move |_| password_visible.toggle(),
                                    if password_visible() { EyeOff {} } else { Eye {} }
                                }
                            }
                        }
                    }
                    if let Some(message) = error() { StatusMessage { error: true, message } }
                    Button { r#type: "submit", disabled: busy(), if busy() { "正在登录" } else { "登录" } }
                }
                Button { r#type: "button", variant: ButtonVariant::Ghost, disabled: busy(), onclick: move |_| registering.set(true), "注册账号" }
            }
            if registering() { super::registration::RegistrationDialog { on_close: move |_| registering.set(false) } }
        }
    }
}
