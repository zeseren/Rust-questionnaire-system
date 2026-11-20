/* @ts-self-types="./logic_wasm.d.ts" */
import * as wasm from "./logic_wasm_bg.wasm";
import { __wbg_set_wasm } from "./logic_wasm_bg.js";

__wbg_set_wasm(wasm);
wasm.__wbindgen_start();
export {
    FlowSession, compile_document_logic, format_logic_text, inspect_logic_text, parse_logic_text
} from "./logic_wasm_bg.js";
