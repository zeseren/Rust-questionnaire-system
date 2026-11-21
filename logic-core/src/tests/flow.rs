//! 流程状态机内部不变量：结构校验、分支数量边界与当前页可见题目的改答约束。

use std::collections::BTreeMap;

use serde_json::json;

use crate::domain::{Answer, Document, DocumentLogic, Question, QuestionKind, QuestionRule};
use crate::flow::{
    CompiledFlow, FlowBranch, FlowDefinition, FlowEvent, FlowNode, FlowOutcome, FlowSession,
    MAX_BRANCHES, Node,
};
use crate::logic::LogicErrorCode;
use crate::tests::support::{id, 版本, 选项};

fn 单选题(value: u128, options: &[u128]) -> Question {
    Question {
        id: id(value),
        title: format!("题目 {value}"),
        required: false,
        kind: QuestionKind::SingleChoice {
            options: options.iter().map(|option| 选项(*option)).collect(),
        },
    }
}

fn 文本题(value: u128) -> Question {
    Question {
        id: id(value),
        title: format!("文本题 {value}"),
        required: false,
        kind: QuestionKind::Text { max_length: 200 },
    }
}

fn 文档(questions: Vec<Question>, logic: Option<DocumentLogic>) -> Document {
    Document {
        logic,
        title: "流程问卷".into(),
        description: String::new(),
        purpose: "收集反馈".into(),
        thank_you: String::new(),
        questions,
    }
}

fn 逻辑(flow: FlowDefinition) -> DocumentLogic {
    逻辑带规则(flow, Vec::new())
}

fn 逻辑带规则(flow: FlowDefinition, rules: Vec<QuestionRule>) -> DocumentLogic {
    DocumentLogic {
        flow: Some(flow),
        dsl_version: 版本(),
        semantics_version: 版本(),
        variables: Vec::new(),
        rules,
        aliases: BTreeMap::new(),
    }
}

fn 页面(value: u128, questions: &[u128], next: u128) -> FlowNode {
    FlowNode::Page {
        id: id(value),
        question_ids: questions.iter().map(|question| id(*question)).collect(),
        next: id(next),
    }
}

fn 结束(value: u128) -> FlowNode {
    FlowNode::End {
        id: id(value),
        outcome: FlowOutcome::Complete,
    }
}

fn 分支流程(branches: Vec<FlowBranch>) -> FlowDefinition {
    FlowDefinition {
        entry: id(201),
        nodes: vec![
            页面(201, &[1], 202),
            FlowNode::Branch {
                id: id(202),
                branches,
                otherwise: id(203),
            },
            结束(203),
        ],
    }
}

fn 编译流程(questions: &[Question], logic: &DocumentLogic) -> CompiledFlow {
    logic
        .compile(questions)
        .unwrap_or_else(|error| panic!("流程应可编译：{error:?}"))
        .flow
        .expect("配置了流程时必须产出编译结果")
}

/// 流程结构拒绝空节点非首页与空页面
#[test]
fn flow_structure_rejects_empty_nodes_non_first_pages_and_empty_pages() {
    let questions = [单选题(1, &[101])];

    let 空节点 = FlowDefinition {
        entry: id(201),
        nodes: Vec::new(),
    };
    let 错误 = 逻辑(空节点).compile(&questions).unwrap_err();
    assert_eq!(错误.code, LogicErrorCode::InvalidFlow);
    assert_eq!(错误.path, "$.logic.flow");

    // 入口必须指向首个页面节点。
    let 入口错误 = FlowDefinition {
        entry: id(202),
        nodes: vec![页面(201, &[1], 202), 结束(202)],
    };
    let 错误 = 逻辑(入口错误).compile(&questions).unwrap_err();
    assert_eq!(错误.code, LogicErrorCode::InvalidFlow);
    assert_eq!(错误.path, "$.logic.flow");
    assert_eq!(错误.question_id, None);

    // 页面不能没有题目。
    let 空页面 = FlowDefinition {
        entry: id(201),
        nodes: vec![页面(201, &[], 202), 结束(202)],
    };
    let 错误 = 逻辑(空页面).compile(&questions).unwrap_err();
    assert_eq!(错误.code, LogicErrorCode::InvalidFlow);
    assert_eq!(错误.path, "$.logic.flow");
    assert_eq!(错误.question_id, None);
}

/// 分支数量与常量条件遵守边界
#[test]
fn branch_count_and_constant_condition_respect_bounds() {
    let questions = [单选题(1, &[101])];
    let 守 = || json!({"op": "answered", "question_id": id(1).to_string()});
    let 真 = || json!({"op": "boolean", "value": true});

    // 空分支列表无法给出默认出口。
    let 错误 = 逻辑(分支流程(Vec::new())).compile(&questions).unwrap_err();
    assert_eq!(错误.code, LogicErrorCode::InvalidFlow);
    assert_eq!(错误.path, "$.logic.flow");

    // 恰好 MAX_BRANCHES 个条件可以编译，且常量 true 允许出现在末位。
    let mut 满分 = (0..MAX_BRANCHES - 1)
        .map(|_| FlowBranch {
            when: 守(),
            target: id(203),
        })
        .collect::<Vec<_>>();
    满分.push(FlowBranch {
        when: 真(),
        target: id(203),
    });
    let flow = 编译流程(&questions, &逻辑(分支流程(满分)));
    match &flow.nodes[1] {
        Node::Branch { guards, otherwise } => {
            assert_eq!(guards.len(), MAX_BRANCHES);
            assert_eq!(*otherwise, 2, "默认出口指向结束节点索引");
        }
        _ => panic!("第二节点必须是分支"),
    }

    // 超过 MAX_BRANCHES 直接拒绝。
    let mut 超限 = (0..MAX_BRANCHES)
        .map(|_| FlowBranch {
            when: 守(),
            target: id(203),
        })
        .collect::<Vec<_>>();
    超限.push(FlowBranch {
        when: 真(),
        target: id(203),
    });
    let 错误 = 逻辑(分支流程(超限)).compile(&questions).unwrap_err();
    assert_eq!(错误.code, LogicErrorCode::InvalidFlow);

    // 常量 true 位于末位仅作为默认判定，允许编译。
    assert!(
        逻辑(分支流程(vec![
            FlowBranch {
                when: 守(),
                target: id(203),
            },
            FlowBranch {
                when: 真(),
                target: id(203),
            },
        ]))
        .compile(&questions)
        .is_ok()
    );
}

/// 改答只接受当前页面的可见题目
#[test]
fn answer_change_accepts_only_visible_questions_on_current_page() {
    // 当前页被显示规则隐藏的题目同样不能改答。
    let 隐藏问卷 = 文档(
        vec![单选题(1, &[101, 102]), 单选题(2, &[103, 104])],
        Some(逻辑带规则(
            FlowDefinition {
                entry: id(201),
                nodes: vec![页面(201, &[1, 2], 202), 结束(202)],
            },
            vec![QuestionRule {
                question_id: id(2),
                visible_if: Some(json!({"op": "boolean", "value": false})),
                required_if: None,
                validate: None,
            }],
        )),
    );
    let mut 会话 = FlowSession::new(&隐藏问卷).unwrap();
    assert_eq!(会话.snapshot().visible, [id(1)].as_slice());
    let 错误 = 会话
        .dispatch(FlowEvent::Change {
            question_id: id(2),
            value: json!(id(103).to_string()),
        })
        .unwrap_err();
    assert_eq!(错误.code, LogicErrorCode::InvalidEvent, "不能改答隐藏题目");
    assert_eq!(错误.path, "$.event");
}

/// 动态必填未知视为不必填且不阻断填写
#[test]
fn unknown_dynamic_required_is_treated_as_optional_and_does_not_block_filling() {
    let questions = vec![单选题(1, &[101, 102]), 文本题(2)];
    let logic = 逻辑带规则(
        FlowDefinition {
            entry: id(201),
            nodes: vec![页面(201, &[1, 2], 202), 结束(202)],
        },
        vec![QuestionRule {
            question_id: id(2),
            visible_if: None,
            required_if: Some(json!({
                "op": "selected",
                "question_id": id(1).to_string(),
                "option_id": id(101).to_string(),
            })),
            validate: None,
        }],
    );
    let 问卷 = 文档(questions, Some(logic));

    // q1 未答时 required_if 未知：q2 不被标记必填，也不记录错误。
    let mut 会话 = FlowSession::new(&问卷).unwrap();
    let snapshot = 会话.snapshot();
    assert!(
        snapshot.errors.is_empty(),
        "未知的 required_if 不能产生错误"
    );
    assert!(!snapshot.required.contains(&id(2)));

    // 未知不阻断填写，输入正常进入答案集合。
    会话
        .dispatch(FlowEvent::Change {
            question_id: id(2),
            value: json!("保留输入"),
        })
        .unwrap();
    assert_eq!(会话.snapshot().answers[&id(2)], json!("保留输入"));
    会话.dispatch(FlowEvent::Next).unwrap();
    assert_eq!(会话.snapshot().outcome, Some(FlowOutcome::Complete));

    // 服务端重建同样把未知的 required_if 当作不必填。
    let evaluated = 会话
        .evaluate_answers(vec![Answer {
            question_id: id(2),
            value: json!("保留输入"),
        }])
        .unwrap();
    assert_eq!(evaluated.outcome, FlowOutcome::Complete);
    assert_eq!(evaluated.answers[&id(2)], json!("保留输入"));

    // q1 明确选中触发选项后，q2 才成为必填。
    let mut 触发 = FlowSession::new(&问卷).unwrap();
    触发
        .dispatch(FlowEvent::Change {
            question_id: id(1),
            value: json!(id(101).to_string()),
        })
        .unwrap();
    assert!(触发.snapshot().required.contains(&id(2)));
}

/// 流程续写身份返回旧新节点映射
#[test]
fn flow_continuation_identity_returns_old_to_new_node_mapping() {
    let mut flow = FlowDefinition {
        entry: id(201),
        nodes: vec![页面(201, &[1], 202), 结束(202)],
    };
    let 旧身份: Vec<_> = flow.nodes.iter().map(FlowNode::id).collect();
    let 映射 = flow.renew_ids(&BTreeMap::new(), &BTreeMap::new(), &BTreeMap::new());
    assert_eq!(映射.len(), 2);
    for 旧 in 旧身份 {
        assert!(映射.contains_key(&旧), "缺少旧节点身份 {旧}");
    }
    assert_eq!(flow.entry, 映射[&id(201)]);
    assert_eq!(flow.nodes[0].id(), 映射[&id(201)]);
    assert_eq!(flow.nodes[1].id(), 映射[&id(202)]);
}
