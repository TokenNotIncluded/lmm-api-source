//! Preserve the legacy rule that a status question never consumes a UTC week.
//! This reads only the original persisted user text, never a model-authored intent.
fn contains(text: &str, terms: &[&str]) -> bool {
    terms.iter().any(|term| text.contains(term))
}
fn completion_question(text: &str) -> bool {
    let operation = [
        "评估", "决定", "申请", "领取", "发放", "发码", "evaluate", "decid", "appl", "claim",
        "issu",
    ]
    .iter()
    .filter_map(|word| text.find(word))
    .min();
    let Some(at) = operation else {
        return false;
    };
    if contains(
        &text[..at],
        &[
            "有没有",
            "是否",
            "是不是",
            "有无",
            "did you",
            "have you",
            "had you",
            "has my",
            "was my",
            "did i",
            "have i",
            "has it",
            "was it",
        ],
    ) {
        return true;
    }
    let operation = &text[at..];
    if contains(operation, &["了吗", "了没", "过吗", "完了吗", "完了没"]) {
        return true;
    }
    let question = contains(operation, &["吗", "么", "没有", "没"]);
    if operation.contains('了') && question {
        return true;
    }
    (text.starts_with('你') || text.starts_with('您') || text.starts_with("you "))
        && (question || operation.contains('?'))
}
fn explicit_decision(text: &str) -> bool {
    !completion_question(text)
        && contains(
            text,
            &[
                "申请",
                "领取",
                "想领",
                "要领",
                "帮我领",
                "给我",
                "送我",
                "发我",
                "希望给",
                "很需要",
                "请评估",
                "帮我评估",
                "评估我",
                "评估一下",
                "现在评估",
                "想评估",
                "我要评估",
                "你来评估",
                "麻烦评估",
                "请决定",
                "帮我决定",
                "决定给我",
                "apply",
                "i want to claim",
                "i'd like to claim",
                "claim my",
                "claim the",
                "please claim",
                "give me",
                "grant me",
                "please evaluate",
                "evaluate my",
                "evaluate me",
                "evaluate the",
                "please decide",
                "decide my",
                "decide the",
            ],
        )
}
fn readonly_followup(text: &str) -> bool {
    if completion_question(text)
        || contains(
            text,
            &[
                "领取了吗",
                "领取了没",
                "领过吗",
                "领了吗",
                "领取成功",
                "是否领取",
                "有没有领取",
                "已经领取",
                "还差什么条件",
                "还需要什么条件",
                "还缺什么条件",
                "算实质交流",
                "算实质性交流",
                "算不算实质",
                "评估过",
                "评估了吗",
                "被评估",
                "决定了吗",
                "决定了没",
                "已经决定",
                "evaluated",
                "decided",
                "evaluation status",
                "decision status",
                "还能领取",
                "可以领取",
                "能领取吗",
                "怎么领",
                "如何领取",
                "怎么领取",
                "have i claimed",
                "did i claim",
                "already claimed",
                "claim successful",
                "what conditions",
                "requirements left",
                "count as substantive",
                "counts as substantive",
                "how do i claim",
                "can i claim",
            ],
        )
    {
        return true;
    }
    !explicit_decision(text)
        && contains(
            text,
            &[
                "规则",
                "条件",
                "资格",
                "标准",
                "当前状态",
                "目前多少",
                "rules",
                "requirements",
                "criteria",
                "eligibility",
                "current status",
            ],
        )
}
fn weekly_status(text: &str) -> bool {
    if !contains(
        text,
        &[
            "优惠券",
            "coupon",
            "每周折扣",
            "本周折扣",
            "周折扣",
            "每周优惠",
            "本周优惠",
            "充值折扣",
            "优惠码",
            "折扣码",
            "weekly discount",
            "weekly coupon",
            "recharge discount",
            "discount code",
        ],
    ) {
        return false;
    }
    if completion_question(text)
        || contains(
            text,
            &[
                "领取了吗",
                "领取了没",
                "领过了吗",
                "领过吗",
                "领了吗",
                "领取成功",
                "领取是否成功",
                "是否领取",
                "有没有领取",
                "已经领取",
                "已领取",
                "申请结果",
                "申请成功",
                "申请过",
                "申请的",
                "评估结果",
                "审核结果",
                "评估过",
                "决定过",
                "评估了吗",
                "被评估",
                "决定了吗",
                "决定了没",
                "已经决定",
                "evaluated",
                "decided",
                "did i claim",
                "have i claimed",
                "already claimed",
                "claim successful",
                "was my claim",
                "application status",
                "application result",
                "evaluation result",
                "decision result",
            ],
        )
    {
        return true;
    }
    if contains(
        text,
        &[
            "求一张",
            "求个",
            "给张",
            "来张",
            "我想申请",
            "我要申请",
            "我希望申请",
            "帮我申请",
            "替我申请",
            "请申请",
            "想领",
            "要领",
            "帮我领",
            "给我优惠码",
            "给我折扣码",
            "发我优惠码",
            "发我折扣码",
            "送我优惠码",
            "送我折扣码",
            "给我本周折扣",
            "给我每周折扣",
            "i want to apply",
            "i'd like to apply",
            "i want to claim",
            "i'd like to claim",
            "please claim",
            "grant me",
            "give me a discount",
            "give me the discount",
        ],
    ) || text.starts_with("apply ")
        || text.starts_with("please apply ")
        || text.starts_with("claim ")
    {
        return false;
    }
    if contains(text, &["评估", "决定", "evaluate", "decide"]) && explicit_decision(text) {
        return false;
    }
    if contains(
        text,
        &[
            "状态",
            "查询",
            "查看",
            "领过",
            "领了",
            "领到",
            "有没有",
            "是否有",
            "还有",
            "审核",
            "通过了",
            "结果",
            "需要申请",
            "需要领取",
            "申请吗",
            "规则",
            "条件",
            "要求",
            "资格",
            "还差",
            "怎么",
            "如何",
            "怎样",
            "能申请",
            "可以申请",
            "能领取",
            "可以领取",
            "能获得",
            "可以获得",
            "status",
            "check",
            "show",
            "do i have",
            "is there",
            "available",
            "rules",
            "requirement",
            "criteria",
            "eligible",
            "eligibility",
            "how do",
            "how to",
            "can i apply",
            "can i claim",
        ],
    ) {
        return true;
    }
    !contains(
        text,
        &[
            "求一张",
            "求个",
            "给张",
            "来张",
            "申请",
            "领取",
            "想领",
            "要领",
            "帮我领",
            "给我",
            "送我",
            "发我",
            "希望给",
            "apply",
            "i want to claim",
            "i'd like to claim",
            "claim my",
            "claim the",
            "please claim",
            "give me",
            "grant me",
        ],
    )
}
pub(super) fn preserves_status_only(original: &str) -> bool {
    let text = original.trim().to_lowercase();
    text.is_empty()
        || contains(
            &text,
            &[
                "不要给",
                "不要发",
                "不想申请",
                "不需要优惠",
                "别发",
                "别给",
                "do not issue",
                "don't issue",
                "do not evaluate",
                "don't evaluate",
                "no thanks",
                "not now",
            ],
        )
        || readonly_followup(&text)
        || weekly_status(&text)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn questions_and_quoted_completed_actions_do_not_consume_week() {
        for text in [
            "本周优惠券还有吗？",
            "请查看我的每周折扣状态",
            "你已经评估我的优惠券了吗？",
            "有没有按照我说的请评估规则评估？",
            "Did you evaluate my weekly coupon already?",
            "Have I claimed the discount code?",
            "还差什么条件才算实质性交流？",
            "How do I claim?",
            "Can I claim the weekly coupon?",
            "不要给我优惠券",
            "Don't evaluate my weekly discount now.",
        ] {
            assert!(preserves_status_only(text), "{text}");
        }
    }
    #[test]
    fn explicit_application_remains_possible_after_two_real_turns() {
        for text in [
            "请根据这些用途评估我本周优惠券的申请",
            "Please evaluate whether I qualify for a weekly discount.",
            "我想申请本周折扣",
            "Give me a weekly coupon",
            "Please apply for the weekly coupon",
        ] {
            assert!(!preserves_status_only(text), "{text}");
        }
    }
}
