use super::http;
use aio_plugin_identity_model::PasswordRequest;
use az_ui_components::{
    admin::{AsyncResult, EditorDialog, PageHeader, PageSurface, RequestState, StatusMessage},
    button::Button,
    input::Input,
};
use dioxus::prelude::*;
use dioxus_icons::lucide::KeyRound;

#[allow(non_snake_case)]
pub(super) fn ProfilePage() -> Element {
    let mut revision = use_signal(|| 0_u64);
    let session = use_resource(move || {
        let _ = revision();
        http::load_session()
    });
    let mut editing = use_signal(|| false);
    let mut saved = use_signal(|| false);
    let value = match session.read().as_ref().cloned() {
        Some(Ok(Some(value))) => value,
        Some(Ok(None)) => {
            return rsx! { PageSurface { RequestState { error: "会话已失效", on_retry: move |_| revision += 1 } } };
        }
        Some(Err(error)) => {
            return rsx! { PageSurface { RequestState { error, on_retry: move |_| revision += 1 } } };
        }
        None => return rsx! { PageSurface { RequestState {} } },
    };
    rsx! {
      div { class: "workbench-settings",
        PageSurface {
            PageHeader { title: "个人资料", detail: value.account.clone(), Button { onclick: move |_| editing.set(true), KeyRound {} "修改密码" } }
            if saved() { StatusMessage { message: "密码已修改，其他会话已退出" } }
            section { class: "admin-section", h2 { "账户信息" }
                dl { class: "admin-details", dt { "账号" } dd { "{value.account}" } dt { "姓名" } dd { "{value.display_name}" } dt { "当前工作区" } dd { "{value.tenant_label}" } }
                details { class: "workbench-technical", summary { "技术详情" } p { "用户 ID：" code { "{value.user_id}" } } }
            }
        }
      }
        if editing() { PasswordEditor { on_close: move |_| editing.set(false), on_saved: move |_| { editing.set(false); saved.set(true); } } }
    }
}

#[component]
fn PasswordEditor(on_close: Callback<()>, on_saved: Callback<()>) -> Element {
    let mut current = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut confirmation = use_signal(String::new);
    rsx! { EditorDialog { title: "修改密码", description: "修改成功后其他会话将失效。", on_close, on_saved,
        save: move |_| -> AsyncResult<()> {
            let payload = PasswordRequest { current_password: current(), new_password: password() }; let confirmation = confirmation();
            Box::pin(async move { if payload.new_password != confirmation { return Err("两次输入的新密码不一致".into()); } http::change_password(payload).await })
        },
        label { class: "admin-field", span { "当前密码" } Input { aria_label: "当前密码", r#type: "password", autocomplete: "current-password", required: true, value: current(), oninput: move |event: FormEvent| current.set(event.value()) } }
        label { class: "admin-field", span { "新密码" } Input { aria_label: "新密码", r#type: "password", autocomplete: "new-password", required: true, minlength: 12, value: password(), oninput: move |event: FormEvent| password.set(event.value()) } }
        label { class: "admin-field", span { "确认新密码" } Input { aria_label: "确认新密码", r#type: "password", autocomplete: "new-password", required: true, minlength: 12, value: confirmation(), oninput: move |event: FormEvent| confirmation.set(event.value()) } }
    } }
}
