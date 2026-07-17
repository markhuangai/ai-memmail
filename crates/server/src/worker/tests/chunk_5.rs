fn fake_decisions(scan: SafetyScanResult, action: OutboundAction) -> FakeDecisionEngine {
    FakeDecisionEngine {
        scan,
        classification: question_classification(),
        decision: AgentDecision {
            action: action.clone(),
            safety_notes: "tested".to_string(),
        },
        rule_decision: AgentDecision {
            action,
            safety_notes: "rule tested".to_string(),
        },
        review: OutboundReviewDecision {
            approved: true,
            reason: "approved".to_string(),
        },
        fail_safety: false,
        fail_classification: false,
        fail_agent: false,
        fail_rule: false,
        fail_review: false,
        hang_agent: false,
        missing_classifier_prompt: false,
        missing_rule_prompt: false,
        context_limit_at: None,
        calls: Arc::new(Mutex::new(DecisionCallCounts::default())),
        reviewed_actions: Arc::new(Mutex::new(Vec::new())),
    }
}

fn rejecting_review_decisions() -> FakeDecisionEngine {
    FakeDecisionEngine {
        scan: safe_scan(),
        classification: question_classification(),
        decision: AgentDecision {
            action: reply_action().clone(),
            safety_notes: "tested".to_string(),
        },
        rule_decision: AgentDecision {
            action: reply_action(),
            safety_notes: "rule tested".to_string(),
        },
        review: OutboundReviewDecision {
            approved: false,
            reason: "unexpected recipient".to_string(),
        },
        fail_safety: false,
        fail_classification: false,
        fail_agent: false,
        fail_rule: false,
        fail_review: false,
        hang_agent: false,
        missing_classifier_prompt: false,
        missing_rule_prompt: false,
        context_limit_at: None,
        calls: Arc::new(Mutex::new(DecisionCallCounts::default())),
        reviewed_actions: Arc::new(Mutex::new(Vec::new())),
    }
}

fn failing_review_decisions() -> FakeDecisionEngine {
    FakeDecisionEngine {
        scan: safe_scan(),
        classification: question_classification(),
        decision: AgentDecision {
            action: reply_action().clone(),
            safety_notes: "tested".to_string(),
        },
        rule_decision: AgentDecision {
            action: reply_action(),
            safety_notes: "rule tested".to_string(),
        },
        review: OutboundReviewDecision {
            approved: true,
            reason: "approved".to_string(),
        },
        fail_safety: false,
        fail_classification: false,
        fail_agent: false,
        fail_rule: false,
        fail_review: true,
        hang_agent: false,
        missing_classifier_prompt: false,
        missing_rule_prompt: false,
        context_limit_at: None,
        calls: Arc::new(Mutex::new(DecisionCallCounts::default())),
        reviewed_actions: Arc::new(Mutex::new(Vec::new())),
    }
}
