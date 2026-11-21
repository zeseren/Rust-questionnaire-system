//! 流程回归：共享拓扑/事件样例、最终提交重建、已存答案与会话公开契约。

use std::collections::BTreeMap;

use questionnaire_logic::domain::{
    Answer, AnswerState, Document, FlowDefinition, FlowEvaluator, FlowEvent, FlowNode, FlowOutcome,
    FlowSession, LogicErrorCode, QuestionKind,
};
use serde_json::{Value, json};
use uuid::Uuid;

mod support;

fn id(value: u32) -> Uuid {
    Uuid::parse_str(&format!("00000000-0000-4000-8000-{value:012}")).unwrap()
}

fn document(name: &str) -> Document {
    serde_json::from_value(support::flow_document(name)).unwrap()
}

#[test]
fn 配置路径与答案身份接受标准格式并拒绝非规范_uuid且选项值仍规范化() {
    let contracts = support::input_contracts();
    let fixtures = support::flows();
    let document_name = contracts["document"].as_str().unwrap();
    let source = &fixtures["documents"][document_name];
    let canonical = contracts["canonical_id"].as_str().unwrap();

    // 配置侧：先同步替换声明和引用，保证标准写法仍构成有效问卷，再逐字段扰动。
    for path in contracts["configuration_paths"].as_array().unwrap() {
        let path = path.as_str().unwrap();
        let original = source.pointer(path).unwrap().as_str().unwrap();
        let renamed = serde_json::to_string(source)
            .unwrap()
            .replace(original, canonical);
        for case in contracts["uuid_cases"].as_array().unwrap() {
            let mut raw: Value = serde_json::from_str(&renamed).unwrap();
            *raw.pointer_mut(path).unwrap() = case["raw"].clone();
            let valid = serde_json::from_value::<Document>(raw)
                .is_ok_and(|document| document.compile_logic().is_ok());
            assert_eq!(
                valid,
                case["configuration_valid"].as_bool().unwrap(),
                "{path}：{}",
                case["name"]
            );
        }
    }

    // 选项答案：值仍是规范化身份，规范写法之外的 UUID 一律拒绝。
    let mut raw = source.clone();
    raw.as_object_mut().unwrap().remove("logic");
    raw["questions"].as_array_mut().unwrap().truncate(1);
    raw["questions"][0]["options"][0]["id"] = canonical.into();
    let document: Document = serde_json::from_value(raw.clone()).unwrap();
    let question_id = document.questions[0].id;
    let evaluator = FlowEvaluator::new(&document).unwrap();
    for case in contracts["uuid_cases"].as_array().unwrap() {
        let value = case["raw"].clone();
        let plain = document.validate_answers(vec![Answer {
            question_id,
            value: value.clone(),
        }]);
        let flow = evaluator.evaluate_answers(vec![Answer { question_id, value }]);
        let expected_valid = case["option_valid"].as_bool().unwrap();
        match (plain, flow) {
            (Ok(answers), Ok(evaluated)) => {
                assert!(expected_valid, "选项答案：{}", case["name"]);
                assert_eq!(answers, evaluated.answers, "{}", case["name"]);
                assert_eq!(answers[&question_id], canonical, "{}", case["name"]);
            }
            (Err(error), Err(failure)) => {
                assert!(!expected_valid, "选项答案：{}", case["name"]);
                assert_eq!(error.question_id, Some(question_id), "{}", case["name"]);
                assert_eq!(
                    failure.code,
                    LogicErrorCode::InvalidAnswer,
                    "{}",
                    case["name"]
                );
                assert_eq!(failure.path, "$.answers", "{}", case["name"]);
                assert_eq!(failure.question_id, Some(question_id), "{}", case["name"]);
            }
            (plain, flow) => panic!(
                "答案入口必须同时成功或失败：{}：{plain:?} / {flow:?}",
                case["name"]
            ),
        }
    }

    // 答案与事件中的题目身份沿用同一套严格 UUID 规则。
    raw["questions"][0] = source["questions"][0].clone();
    raw["questions"][0]["id"] = canonical.into();
    let document: Document = serde_json::from_value(raw).unwrap();
    let option_value = source["questions"][0]["options"][0]["id"].clone();
    for case in contracts["uuid_cases"].as_array().unwrap() {
        let valid = case["configuration_valid"].as_bool().unwrap();
        let answer = json!({ "question_id": case["raw"], "value": option_value.clone() });
        let accepted = serde_json::from_value::<Answer>(answer)
            .is_ok_and(|answer| document.validate_answers(vec![answer]).is_ok());
        assert_eq!(accepted, valid, "答案题目身份：{}", case["name"]);

        let event = json!({
            "type": "change",
            "question_id": case["raw"],
            "value": option_value.clone(),
        });
        let mut session = FlowSession::new(&document).unwrap();
        let accepted = serde_json::from_value::<FlowEvent>(event.clone())
            .is_ok_and(|event| session.dispatch(event).is_ok());
        assert_eq!(accepted, valid, "事件题目身份：{}", case["name"]);

        if !valid {
            // 非规范身份在 DTO 解码层就被拒绝，无法进入会话；这里再用合法但不在当前页的
            // 身份单独固定会话自身的拒绝分类与路径。
            assert!(
                serde_json::from_value::<FlowEvent>(event).is_err(),
                "非规范题目身份必须先在事件解码层被拒绝：{}",
                case["name"]
            );
            let error = session
                .dispatch(FlowEvent::Change {
                    question_id: id(999),
                    value: option_value.clone(),
                })
                .unwrap_err();
            assert_eq!(error.code, LogicErrorCode::InvalidEvent, "{}", case["name"]);
            assert_eq!(error.path, "$.event", "{}", case["name"]);
        }
    }
}

#[test]
fn 共享流程拒绝非法拓扑和未保护的分支依赖() {
    let fixtures = support::flows();
    for case in fixtures["compilation"].as_array().unwrap() {
        let mut document = support::flow_document(case["document"].as_str().unwrap());
        for edit in case["edits"].as_array().unwrap() {
            let mut target = &mut document;
            for segment in edit["path"].as_array().unwrap() {
                target = match segment {
                    Value::String(key) => &mut target[key],
                    Value::Number(index) => {
                        let position = usize::try_from(index.as_u64().unwrap()).unwrap();
                        &mut target[position]
                    }
                    _ => panic!("样例路径只能包含字段或索引"),
                };
            }
            *target = edit["value"].clone();
        }
        let document: Document = serde_json::from_value(document).unwrap();
        let actual = document.compile_logic().err().map(|failure| failure.code);
        assert_eq!(json!(actual), case["error"], "编译样例：{}", case["name"]);
    }
}

#[test]
fn 流程节点数量上限允许_256_并拒绝_257() {
    // 正向边界：节点总数恰好 256。页面必须逐题覆盖且不能为空，而文档题目上限为 100，
    // 因此 255 个页面不可达；这里用 4 个题目页面 + 251 个自动分支 + 1 个结束节点
    // 恰好填满 256 个节点，仍然固定 MAX_FLOW_NODES 的正向边界。
    let mut document = document("diamond");
    let question_count = document.questions.len();
    let mut nodes: Vec<FlowNode> = Vec::with_capacity(256);
    for (index, question) in document.questions.iter().enumerate() {
        nodes.push(FlowNode::Page {
            id: Uuid::from_u128(1_000 + index as u128),
            question_ids: vec![question.id],
            next: Uuid::from_u128(1_001 + index as u128),
        });
    }
    for index in question_count..255 {
        let next = Uuid::from_u128(1_001 + index as u128);
        nodes.push(FlowNode::Branch {
            id: Uuid::from_u128(1_000 + index as u128),
            branches: vec![questionnaire_logic::domain::FlowBranch {
                when: json!({ "op": "boolean", "value": true }),
                target: next,
            }],
            otherwise: next,
        });
    }
    nodes.push(FlowNode::End {
        id: Uuid::from_u128(1_000 + 255),
        outcome: FlowOutcome::Complete,
    });
    document.logic.as_mut().unwrap().flow = Some(FlowDefinition {
        entry: Uuid::from_u128(1_000),
        nodes,
    });
    assert_eq!(
        document
            .logic
            .as_ref()
            .unwrap()
            .flow
            .as_ref()
            .unwrap()
            .nodes
            .len(),
        256,
        "正向边界必须恰好包含 256 个节点"
    );
    document
        .compile_logic()
        .expect("流程节点恰好等于上限应编译通过")
        .expect("规则包必须存在");

    // 257 个节点超过 256 上限；用循环构造，替代手写的大流程样例。
    let mut document = self::document("diamond");
    let nodes = (0..257)
        .map(|index| FlowNode::End {
            id: Uuid::from_u128(1_000 + index),
            outcome: FlowOutcome::Complete,
        })
        .collect();
    document.logic.as_mut().unwrap().flow = Some(FlowDefinition {
        entry: Uuid::from_u128(1_000),
        nodes,
    });
    let failure = FlowEvaluator::new(&document).unwrap_err();
    assert_eq!(failure.code, LogicErrorCode::LimitExceeded);
    assert_eq!(failure.path, "$.logic.flow");
    assert_eq!(failure.question_id, None);
}

#[test]
fn 共享事件序列保持路径清理回退和题型规范化一致() {
    let fixtures = support::flows();
    for case in fixtures["sessions"].as_array().unwrap() {
        let document: Document =
            serde_json::from_value(support::flow_document(case["document"].as_str().unwrap()))
                .unwrap();
        let mut session = FlowSession::new(&document).unwrap();
        for (index, step) in case["steps"].as_array().unwrap().iter().enumerate() {
            let before = serde_json::to_value(session.snapshot()).unwrap();
            if let Some(event) = step["event_json"].as_str() {
                let event: FlowEvent = serde_json::from_str(event).unwrap();
                let actual = session.dispatch(event).err().map(|failure| failure.code);
                assert_eq!(
                    json!(actual),
                    step["error"],
                    "事件样例：{}，第 {index} 步",
                    case["name"]
                );
                if actual.is_some() {
                    assert_eq!(
                        serde_json::to_value(session.snapshot()).unwrap(),
                        before,
                        "失败事件不能改变状态"
                    );
                }
            }
            let actual = serde_json::to_value(session.snapshot()).unwrap();
            for (key, expected) in step["expected"].as_object().unwrap() {
                assert!(
                    equivalent(&actual[key], expected),
                    "快照样例：{}，第 {index} 步，{key}：实际 {}，预期 {expected}",
                    case["name"],
                    actual[key]
                );
            }
        }
    }
}

#[test]
fn 最终提交样例只通过流程求值重建答卷() {
    let submissions = support::submissions();
    for case in submissions["cases"].as_array().unwrap() {
        let document: Document =
            serde_json::from_value(support::flow_document(case["document"].as_str().unwrap()))
                .unwrap();
        let answers: Vec<Answer> =
            serde_json::from_str(case["answers_json"].as_str().unwrap()).unwrap();
        let actual = match FlowEvaluator::new(&document)
            .unwrap()
            .evaluate_answers(answers)
        {
            Ok(value) => json!({ "value": value }),
            Err(error) => json!({"error": {
                "code": error.code, "question_id": error.question_id,
            }}),
        };
        assert!(
            equivalent(&actual, &case["expected"]),
            "提交样例：{}，实际 {actual}，预期 {}",
            case["name"],
            case["expected"]
        );
    }
}

#[test]
fn 已存答案违反流程契约时返回对应错误路径() {
    let mut skipped = BTreeMap::new();
    skipped.insert(id(1), json!(id(101).to_string()));
    skipped.insert(id(3), json!("文本"));

    // 已存答案落在被跳过的题目上仍然拒绝，路径定位到答案集合。
    let document = self::document("diamond");
    let evaluator = FlowEvaluator::new(&document).unwrap();
    let error = evaluator.evaluate_stored_states(&skipped).unwrap_err();
    assert_eq!(error.path, "$.answers");

    // 筛选结束不再是错误：三态重建成功，完整求值返回 screen_out。
    let mut screened = BTreeMap::new();
    screened.insert(id(1), json!(id(102).to_string()));
    let priority = self::document("priority");
    let evaluator = FlowEvaluator::new(&priority).unwrap();
    let (answers, states) = evaluator.evaluate_stored_states(&screened).unwrap();
    assert_eq!(answers, screened);
    assert_eq!(states[&id(1)], AnswerState::Answered);
    let evaluated = evaluator
        .evaluate_answers(vec![Answer {
            question_id: id(1),
            value: json!(id(102).to_string()),
        }])
        .expect("筛选结束必须正常返回");
    assert_eq!(evaluated.outcome, FlowOutcome::ScreenOut);
    assert_eq!(
        evaluated.path.last(),
        Some(&Uuid::parse_str("00000000-0000-4000-8000-000000001007").unwrap())
    );
}

#[test]
fn 填写会话公开交互且不复用或改写外部输入() {
    let document = document("diamond");
    let mut session = FlowSession::new(&document).unwrap();

    // 填完分叉前的题目后进入合流页，动态必填缺失必须由 next 拒绝。
    session
        .dispatch(FlowEvent::Change {
            question_id: id(1),
            value: json!(id(101).to_string()),
        })
        .unwrap();
    session.dispatch(FlowEvent::Next).unwrap();
    session
        .dispatch(FlowEvent::Change {
            question_id: id(2),
            value: json!(3),
        })
        .unwrap();
    session.dispatch(FlowEvent::Next).unwrap();
    let error = session.dispatch(FlowEvent::Next).unwrap_err();
    assert_eq!(error.code, LogicErrorCode::RequiredAnswer);
    assert_eq!(error.question_id, Some(id(4)));

    // 外部答案重建不修改会话，也不改写调用方提供的 Vec。
    let mut branched = FlowSession::new(&document).unwrap();
    branched
        .dispatch(FlowEvent::Change {
            question_id: id(1),
            value: json!(id(102).to_string()),
        })
        .unwrap();
    let before = serde_json::to_vec(&branched.snapshot()).unwrap();
    let input = vec![Answer {
        question_id: id(1),
        value: json!(id(101).to_string()),
    }];
    let original = input.clone();
    branched.evaluate_answers(input.clone()).unwrap();
    assert_eq!(input, original, "求值不能改写调用方提供的答案");
    assert_eq!(serde_json::to_vec(&branched.snapshot()).unwrap(), before);
}

#[test]
fn 校验细节进入快照_next_与提交错误并在更正后清除() {
    let mut document = self::document("diamond");
    let number = document
        .questions
        .iter_mut()
        .find(|question| question.id == id(2))
        .expect("diamond 必须包含数字题");
    number.kind = QuestionKind::Number {
        integer: true,
        min: Some(0.0),
        max: Some(10.0),
    };

    let mut session = FlowSession::new(&document).unwrap();
    session
        .dispatch(FlowEvent::Change {
            question_id: id(1),
            value: json!(id(101).to_string()),
        })
        .unwrap();
    session.dispatch(FlowEvent::Next).unwrap();

    // 整数要求的原因同时出现在快照与 next 错误中，错误码和位置保持稳定。
    session
        .dispatch(FlowEvent::Change {
            question_id: id(2),
            value: json!(3.5),
        })
        .unwrap();
    let snapshot = session.snapshot();
    assert_eq!(snapshot.errors[&id(2)], LogicErrorCode::InvalidAnswer);
    assert_eq!(snapshot.error_messages[&id(2)], "此题只接受整数");
    assert!(!snapshot.answers.contains_key(&id(2)));
    let wire = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(wire["errors"][id(2).to_string().as_str()], "invalid_answer");
    assert_eq!(
        wire["error_messages"][id(2).to_string().as_str()],
        "此题只接受整数"
    );
    let error = session.dispatch(FlowEvent::Next).unwrap_err();
    assert_eq!(error.code, LogicErrorCode::InvalidAnswer);
    assert_eq!(error.path, "$.answers");
    assert_eq!(error.question_id, Some(id(2)));
    assert_eq!(error.message.as_ref(), "此题只接受整数");

    // 服务端重建使用同一条说明，路径指向答案集合。
    let evaluator = FlowEvaluator::new(&document).unwrap();
    let error = evaluator
        .evaluate_answers(vec![
            Answer {
                question_id: id(1),
                value: json!(id(101).to_string()),
            },
            Answer {
                question_id: id(2),
                value: json!(3.5),
            },
        ])
        .unwrap_err();
    assert_eq!(error.code, LogicErrorCode::InvalidAnswer);
    assert_eq!(error.path, "$.answers");
    assert_eq!(error.question_id, Some(id(2)));
    assert_eq!(error.message.as_ref(), "此题只接受整数");

    // 闭区间越界报告实际上下界。
    session
        .dispatch(FlowEvent::Change {
            question_id: id(2),
            value: json!(11),
        })
        .unwrap();
    assert_eq!(
        session.snapshot().error_messages[&id(2)],
        "数值必须在 0 到 10 之间"
    );

    // 更正后快照不再保留旧说明。
    session
        .dispatch(FlowEvent::Change {
            question_id: id(2),
            value: json!(3),
        })
        .unwrap();
    let snapshot = session.snapshot();
    assert!(!snapshot.errors.contains_key(&id(2)));
    assert!(!snapshot.error_messages.contains_key(&id(2)));
    assert_eq!(snapshot.answers[&id(2)].as_f64(), Some(3.0));
    session.dispatch(FlowEvent::Next).unwrap();

    // 文本上限的具体字符数与 next、提交错误保持一致，且不回显答案内容。
    let 超长 = "隐私".repeat(101);
    session
        .dispatch(FlowEvent::Change {
            question_id: id(4),
            value: json!(超长),
        })
        .unwrap();
    let snapshot = session.snapshot();
    let message = &snapshot.error_messages[&id(4)];
    assert_eq!(message, "文本答案最多允许 200 个 Unicode 字符");
    assert!(!message.contains("隐私"), "错误说明不能回显答案内容");
    let error = session.dispatch(FlowEvent::Next).unwrap_err();
    assert_eq!(error.question_id, Some(id(4)));
    assert_eq!(
        error.message.as_ref(),
        "文本答案最多允许 200 个 Unicode 字符"
    );
    let error = evaluator
        .evaluate_answers(vec![
            Answer {
                question_id: id(1),
                value: json!(id(101).to_string()),
            },
            Answer {
                question_id: id(2),
                value: json!(3),
            },
            Answer {
                question_id: id(4),
                value: json!(超长),
            },
        ])
        .unwrap_err();
    assert_eq!(error.path, "$.answers");
    assert_eq!(error.question_id, Some(id(4)));
    assert_eq!(
        error.message.as_ref(),
        "文本答案最多允许 200 个 Unicode 字符"
    );
}

#[test]
fn 题目隐藏后不保留答案校验说明() {
    let document = self::document("rollback");
    let mut session = FlowSession::new(&document).unwrap();

    // rollback 的 q2 受 q1=甲 动态必填；q2 变量用于 q3 的 visible_if（阈值 > 0），
    // 因此必须先选 q1，再设置 q2，q3 才会真正可见。
    session
        .dispatch(FlowEvent::Change {
            question_id: id(1),
            value: json!(id(101).to_string()),
        })
        .unwrap();
    session
        .dispatch(FlowEvent::Change {
            question_id: id(2),
            value: json!(1),
        })
        .unwrap();
    assert!(
        session.snapshot().visible.contains(&id(3)),
        "q2 变量大于 0 时 q3 必须可见"
    );

    // 超长文本先留下具体说明。
    session
        .dispatch(FlowEvent::Change {
            question_id: id(3),
            value: json!("字".repeat(201)),
        })
        .unwrap();
    let snapshot = session.snapshot();
    assert_eq!(snapshot.errors[&id(3)], LogicErrorCode::InvalidAnswer);
    assert_eq!(
        snapshot.error_messages[&id(3)],
        "文本答案最多允许 200 个 Unicode 字符"
    );

    // 改回 0 后 q3 隐藏，错误与说明一并消失，输入也不再保留。
    session
        .dispatch(FlowEvent::Change {
            question_id: id(2),
            value: json!(0),
        })
        .unwrap();
    let snapshot = session.snapshot();
    assert!(!snapshot.visible.contains(&id(3)));
    assert!(!snapshot.errors.contains_key(&id(3)));
    assert!(!snapshot.error_messages.contains_key(&id(3)));
    assert!(!snapshot.answers.contains_key(&id(3)));
}

#[test]
fn 动态必填未知按不必填重建答卷() {
    let document = document("dynamic");
    let mut session = FlowSession::new(&document).unwrap();
    let snapshot = session.snapshot();
    assert!(snapshot.errors.is_empty(), "未知 required_if 不产生错误");
    assert!(!snapshot.required.contains(&id(2)));

    session
        .dispatch(FlowEvent::Change {
            question_id: id(2),
            value: json!("保留输入"),
        })
        .unwrap();
    assert_eq!(session.snapshot().answers[&id(2)], json!("保留输入"));
    session.dispatch(FlowEvent::Next).unwrap();
    assert_eq!(session.snapshot().outcome, Some(FlowOutcome::Complete));

    let evaluated = FlowEvaluator::new(&document)
        .unwrap()
        .evaluate_answers(vec![Answer {
            question_id: id(2),
            value: json!("保留输入"),
        }])
        .unwrap();
    assert_eq!(evaluated.outcome, FlowOutcome::Complete);
    assert_eq!(evaluated.answers[&id(2)], json!("保留输入"));
}

// JSON 的整数与浮点表示不影响数值语义，其他字段仍严格比较。
fn equivalent(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && b.iter()
                    .all(|(key, value)| a.get(key).is_some_and(|actual| equivalent(actual, value)))
        }
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| equivalent(a, b))
        }
        _ => actual == expected,
    }
}
