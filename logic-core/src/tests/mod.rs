//! `logic-core` 单元测试，按生产模块的职责边界拆分。
//!
//! | 模块 | 覆盖的边界 |
//! | --- | --- |
//! | `identity` | 非零标准连字符 UUID 的严格解析与反序列化 |
//! | `domain` | 长度/空字符校验、草稿数值边界与各题型答案规范化 |
//! | `logic` | 值类型、数字常量、选择器、答案读取与存在性守护分析 |
//! | `logic_document` | 空规则包、同层变量排序、身份重写与变量/规则上限 |
//! | `flow` | 流程结构边界、分支上限与隐藏题目改答拦截 |
//! | `logic_text` | 字节偏移行列定位、词法预算限制与巡检绑定扫描 |

mod domain;
mod flow;
mod identity;
mod logic;
mod logic_document;
mod logic_text;
