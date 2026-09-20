use crate::http;
use aio_plugin_identity_model::{LedgerEntry, RechargeRequest, SubscribeRequest, WalletView};
use az_ui_components::{
    admin::{AsyncResult, EditorDialog, StatusMessage},
    button::{Button, ButtonSize, ButtonVariant},
    dialog::{Dialog, DialogDescription, DialogTitle},
    input::Input,
};
use dioxus::prelude::*;
use dioxus_icons::lucide::{CreditCard, RefreshCw, Sparkles, Wallet};

/// 钱包区块：余额、当前套餐、周期额度、充值、订阅和最近账本。
#[component]
pub(crate) fn WalletSection() -> Element {
    let mut revision = use_signal(|| 0_u64);
    let wallet = use_resource(move || {
        let _ = revision();
        http::load_wallet()
    });
    let ledger = use_resource(move || {
        let _ = revision();
        http::load_ledger(None)
    });
    let mut recharging = use_signal(|| false);
    let mut subscribing = use_signal(|| false);
    let mut confirming = use_signal(|| None::<String>);
    let mut status = use_signal(|| None::<Result<String, String>>);
    let value = wallet.read().as_ref().cloned();
    rsx! {
        section { class: "admin-section", h2 { Wallet {} "钱包" }
            if let Some(message) = status() {
                StatusMessage { error: message.is_err(), message: message.unwrap_or_else(|message| message) }
            }
            match value {
                Some(Ok(wallet)) => rsx! { WalletSummary {
                    wallet: wallet.clone(),
                    on_recharge: move |_| recharging.set(true),
                    on_subscribe: move |_| subscribing.set(true),
                    on_confirm: move |order_id: String| confirming.set(Some(order_id)),
                } },
                Some(Err(error)) => rsx! { StatusMessage { error: true, message: error } },
                None => rsx! { p { class: "admin-meta", "正在加载钱包" } },
            }
            h2 { "最近账本" }
            match ledger.read().as_ref().cloned() {
                Some(Ok(page)) if page.entries.is_empty() => rsx! { p { class: "admin-meta", "暂无账本记录" } },
                Some(Ok(page)) => rsx! { LedgerList { entries: page.entries, on_confirm: move |order_id: String| confirming.set(Some(order_id)) } },
                Some(Err(error)) => rsx! { StatusMessage { error: true, message: error } },
                None => rsx! { p { class: "admin-meta", "正在加载账本" } },
            }
            Button { variant: ButtonVariant::Ghost, size: ButtonSize::IconSm, title: "刷新钱包", aria_label: "刷新钱包",
                onclick: move |_| revision += 1, RefreshCw {} }
        }
        if recharging() {
            RechargeDialog {
                on_close: move |_| recharging.set(false),
                on_created: move |order_id: String| {
                    recharging.set(false);
                    confirming.set(Some(order_id));
                },
            }
        }
        if subscribing() {
            SubscribeDialog {
                on_close: move |_| subscribing.set(false),
                on_saved: move |_| {
                    subscribing.set(false);
                    status.set(Some(Ok("已订阅套餐".into())));
                    revision += 1;
                },
            }
        }
        if let Some(order_id) = confirming() {
            ConfirmRechargeDialog {
                order_id,
                on_close: move |_| confirming.set(None),
                on_confirmed: move |_| {
                    confirming.set(None);
                    status.set(Some(Ok("充值已到账".into())));
                    revision += 1;
                },
            }
        }
    }
}

#[component]
fn WalletSummary(
    wallet: WalletView,
    on_recharge: Callback<()>,
    on_subscribe: Callback<()>,
    on_confirm: Callback<String>,
) -> Element {
    let currency = wallet.currency.clone();
    let balance = format_micros(wallet.balance_micros, &currency);
    rsx! {
        dl { class: "admin-details",
            dt { "余额" } dd { class: "admin-row-title", Wallet {} strong { "{balance}" } }
            dt { "币种" } dd { "{wallet.currency}" }
            dt { "套餐" } dd {
                match &wallet.subscription {
                    Some(subscription) => rsx! { span { class: "admin-status", "data-enabled": (subscription.status == "active").to_string(),
                        Sparkles {} "{subscription.plan.title} · {subscription.status}" } },
                    None => rsx! { span { class: "admin-meta", "未订阅" } },
                }
            }
        }
        if let Some(subscription) = &wallet.subscription {
            p { class: "admin-meta", "周期 {subscription.period_start} 至 {subscription.period_end}" }
            if !subscription.plan.features.is_empty() {
                div { class: "admin-badges",
                    for feature in &subscription.plan.features { span { class: "admin-status", "{feature}" } }
                }
            }
        }
        if !wallet.grants.is_empty() {
            table { class: "data-table",
                caption { class: "admin-sr-only", "周期额度" }
                thead { tr { th { scope: "col", "资源" } th { scope: "col", "已用" } th { scope: "col", "额度" } th { scope: "col", "到期" } } }
                tbody {
                    for grant in &wallet.grants {
                        tr {
                            td { "{grant.resource}" }
                            td { "{grant.consumed}" }
                            td { "{grant.quantity}" }
                            td { time { datetime: grant.period_end.clone(), "{grant.period_end}" } }
                        }
                    }
                }
            }
        }
        div { class: "admin-actions",
            Button { onclick: move |_| on_recharge.call(()), CreditCard {} "充值" }
            Button { variant: ButtonVariant::Outline, onclick: move |_| on_subscribe.call(()), Sparkles {} "订阅套餐" }
        }
    }
}

#[component]
fn LedgerList(entries: Vec<LedgerEntry>, on_confirm: Callback<String>) -> Element {
    rsx! {
        table { class: "data-table",
            caption { class: "admin-sr-only", "账本" }
            thead { tr { th { scope: "col", "时间" } th { scope: "col", "类型" } th { scope: "col", "金额" } th { scope: "col", "说明" } th { scope: "col", "操作" } } }
            tbody {
                for entry in entries {
                    tr {
                        td { time { datetime: entry.created_at.clone(), "{entry.created_at}" } }
                        td { "{entry.kind}" }
                        td { "{entry.amount_micros}" }
                        td { "{entry.description}" }
                        td {
                            if entry.kind == "recharge" {
                                Button { size: ButtonSize::IconSm, variant: ButtonVariant::Ghost, title: "确认充值 {entry.id}", aria_label: "确认充值 {entry.id}",
                                    onclick: move |_| on_confirm.call(entry.id.clone()), "确认" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn RechargeDialog(on_close: Callback<()>, on_created: Callback<String>) -> Element {
    let mut amount = use_signal(|| "60".to_owned());
    rsx! { EditorDialog { title: "充值", description: "创建充值订单，确认到账后计入余额。", on_close, submit_label: "创建订单",
        on_saved: move |_| {},
        save: move |_| -> AsyncResult<()> {
            let parsed = parse_amount(amount());
            Box::pin(async move {
                let amount_micros = parsed?;
                let order = http::create_recharge_order(RechargeRequest {
                    amount_micros,
                    provider: "manual".to_owned(),
                })
                .await?;
                on_created.call(order.id);
                Ok(())
            })
        },
        label { class: "admin-field", span { "金额（USD）" }
            Input { aria_label: "充值金额", r#type: "number", min: "1", step: "1", required: true,
                value: amount(), oninput: move |event: FormEvent| amount.set(event.value()) }
            small { "示例：60 表示 60.00 美元" }
        }
    } }
}

#[component]
fn SubscribeDialog(on_close: Callback<()>, on_saved: Callback<()>) -> Element {
    let mut plan = use_signal(|| "agent_pro_60".to_owned());
    rsx! { EditorDialog { title: "订阅套餐", description: "订阅后立即生效，当前周期按套餐额度计费。", on_close, on_saved, submit_label: "订阅",
        save: move |_| -> AsyncResult<()> {
            let plan_code = plan();
            Box::pin(async move {
                http::subscribe(SubscribeRequest { plan_code }).await.map(|_| ())
            })
        },
        label { class: "admin-field", span { "套餐代码" }
            Input { aria_label: "套餐代码", required: true, value: plan(), oninput: move |event: FormEvent| plan.set(event.value()) }
            small { "默认 agent_pro_60：60 美元 / 月" }
        }
    } }
}

#[component]
fn ConfirmRechargeDialog(
    order_id: String,
    on_close: Callback<()>,
    on_confirmed: Callback<()>,
) -> Element {
    let mut error = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);
    rsx! {
        Dialog { open: true, on_open_change: move |open: bool| { if !open && !busy() { on_close.call(()); } },
            DialogTitle { "确认充值" }
            DialogDescription { "确认后将按订单金额入账。订单：{order_id}" }
            if let Some(message) = error() { StatusMessage { error: true, message } }
            footer { class: "admin-form-footer",
                Button { r#type: "button", variant: ButtonVariant::Outline, disabled: busy(), onclick: move |_| on_close.call(()), "取消" }
                Button { r#type: "button", disabled: busy(), onclick: move |_| {
                    let id = order_id.clone();
                    busy.set(true); error.set(None);
                    spawn(async move {
                        let result = http::confirm_recharge_order(&id).await;
                        busy.set(false);
                        match result { Ok(_) => on_confirmed.call(()), Err(message) => error.set(Some(message)) }
                    });
                }, if busy() { "确认中" } else { "确认到账" } }
            }
        }
    }
}

/// 把用户输入的金额（货币单位）转成微单位整数。
fn parse_amount(value: String) -> Result<i64, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("金额不能为空".into());
    }
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if fraction.len() > 6 || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("金额最多 6 位小数".into());
    }
    let whole = if whole.is_empty() { "0" } else { whole };
    if !whole.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("金额格式无效".into());
    }
    let whole: i64 = whole.parse().map_err(|_| "金额过大".to_owned())?;
    let mut fraction = fraction.to_owned();
    while fraction.len() < 6 {
        fraction.push('0');
    }
    let fraction: i64 = fraction.parse().map_err(|_| "金额格式无效".to_owned())?;
    let micros = whole
        .checked_mul(1_000_000)
        .and_then(|value| value.checked_add(fraction))
        .ok_or_else(|| "金额过大".to_owned())?;
    if micros <= 0 {
        return Err("金额必须大于 0".into());
    }
    Ok(micros)
}

/// 微单位金额格式化为 `12.34 USD` 形式。
fn format_micros(amount_micros: i64, currency: &str) -> String {
    let sign = if amount_micros < 0 { "-" } else { "" };
    let amount = amount_micros.unsigned_abs();
    format!(
        "{sign}{}.{:02} {currency}",
        amount / 1_000_000,
        (amount % 1_000_000) / 10_000
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_decimal_amounts_into_micros() {
        assert_eq!(parse_amount("60".into()).unwrap(), 60_000_000);
        assert_eq!(parse_amount("0.5".into()).unwrap(), 500_000);
        assert_eq!(parse_amount("1.234567".into()).unwrap(), 1_234_567);
        assert_eq!(parse_amount(".5".into()).unwrap(), 500_000);
        assert!(parse_amount("".into()).is_err());
        assert!(parse_amount("0".into()).is_err());
        assert!(parse_amount("-1".into()).is_err());
        assert!(parse_amount("1.2345678".into()).is_err());
        assert!(parse_amount("abc".into()).is_err());
    }

    #[test]
    fn formats_micros_for_display() {
        assert_eq!(format_micros(60_000_000, "USD"), "60.00 USD");
        assert_eq!(format_micros(1_500_000, "USD"), "1.50 USD");
        assert_eq!(format_micros(1, "USD"), "0.00 USD");
        assert_eq!(format_micros(-2_500_000, "USD"), "-2.50 USD");
    }
}
