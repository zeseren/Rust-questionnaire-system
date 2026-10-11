/* @ts-self-types="./logic_wasm.d.ts" */
import * as wasm from "./logic_wasm_bg.wasm";
import { __wbg_set_wasm } from "./logic_wasm_bg.js";

__wbg_set_wasm(wasm);

export {
    FlowSession, compile_document_logic, compile_expression, format_logic_text, parse_logic_text, renew_document_ids
} from "./logic_wasm_bg.js";
