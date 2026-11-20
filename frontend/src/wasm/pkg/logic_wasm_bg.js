/**
 * 有状态的填写会话：文档与状态留在 wasm 线性内存，避免每次改答重编译。
 *
 * 核心的 `FlowSession` 在构造时接管 `Document` 的所有权，导出类只需持有
 * 会话本身，借用生命周期与指针稳定性都由核心类型自行保证。
 *
 * 仅在 wasm32 目标导出：wasm-bindgen 的类导出 wrapper 在原生目标会 panic
 * （“function not implemented on non-wasm32 targets”），纯函数无此限制。
 */
export class FlowSession {
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        FlowSessionFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_flowsession_free(ptr, 0);
    }
    /**
     * 应用一个流程事件；事件 JSON 反序列化失败或流程拒绝时返回 error 信封。
     * @param {string} event_json
     * @returns {string}
     */
    dispatch(event_json) {
        let deferred2_0;
        let deferred2_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16) >>> 0;
            const ptr0 = passStringToWasm0(event_json, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len0 = WASM_VECTOR_LEN;
            wasm.flowsession_dispatch(retptr, this.__wbg_ptr, ptr0, len0);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred2_0 = r0;
            deferred2_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export3(deferred2_0, deferred2_1, 1);
        }
    }
    /**
     * 解析文档、接管所有权并创建空会话；失败时抛出携带 `{"error":…}` 信封 JSON
     * 的字符串（与其它导出同一信封形状）。
     * @param {string} document_json
     */
    constructor(document_json) {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16) >>> 0;
            const ptr0 = passStringToWasm0(document_json, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            const len0 = WASM_VECTOR_LEN;
            wasm.flowsession_new(retptr, ptr0, len0);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var r2 = getDataViewMemory0().getInt32(retptr + 4 * 2, true);
            if (r2) {
                throw takeFromExternrefTable0(r1);
            }
            this.__wbg_ptr = r0;
            FlowSessionFinalization.register(this, this.__wbg_ptr, this);
            return this;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * 只读快照，序列化为 `{"ok":snapshot}`；借用会话集合，避免每次改答深拷贝。
     * @returns {string}
     */
    snapshot() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16) >>> 0;
            wasm.flowsession_snapshot(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export3(deferred1_0, deferred1_1, 1);
        }
    }
}
if (Symbol.dispose) FlowSession.prototype[Symbol.dispose] = FlowSession.prototype.free;

/**
 * 保存前的诊断预检：编译整份规则，只关心错误里的 `path` / `question_id`。
 *
 * 成功信封同时返回 `variable_order`：计算变量的拓扑顺序，供前端在编译期展示
 * 与断言使用。
 * @param {string} logic_json
 * @param {string} questions_json
 * @returns {string}
 */
export function compile_document_logic(logic_json, questions_json) {
    let deferred3_0;
    let deferred3_1;
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16) >>> 0;
        let ptr0, len0, ptr1, len1;
        try {
            ptr0 = passStringToWasm0(logic_json, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            len0 = WASM_VECTOR_LEN;
            ptr1 = passStringToWasm0(questions_json, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            len1 = WASM_VECTOR_LEN;
        } catch (e) {
            if (ptr1) { wasm.__wbindgen_export3(ptr1, len1 * 1, 1); }
            if (ptr0) { wasm.__wbindgen_export3(ptr0, len0 * 1, 1); }
            throw e;
        }
        wasm.compile_document_logic(retptr, ptr0, len0, ptr1, len1);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        deferred3_0 = r0;
        deferred3_1 = r1;
        return getStringFromWasm0(r0, r1);
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
        wasm.__wbindgen_export3(deferred3_0, deferred3_1, 1);
    }
}

/**
 * 把规则 JSON 格式化回文本 DSL。
 * @param {string} logic_json
 * @param {string} questions_json
 * @returns {string}
 */
export function format_logic_text(logic_json, questions_json) {
    let deferred3_0;
    let deferred3_1;
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16) >>> 0;
        let ptr0, len0, ptr1, len1;
        try {
            ptr0 = passStringToWasm0(logic_json, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            len0 = WASM_VECTOR_LEN;
            ptr1 = passStringToWasm0(questions_json, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            len1 = WASM_VECTOR_LEN;
        } catch (e) {
            if (ptr1) { wasm.__wbindgen_export3(ptr1, len1 * 1, 1); }
            if (ptr0) { wasm.__wbindgen_export3(ptr0, len0 * 1, 1); }
            throw e;
        }
        wasm.format_logic_text(retptr, ptr0, len0, ptr1, len1);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        deferred3_0 = r0;
        deferred3_1 = r1;
        return getStringFromWasm0(r0, r1);
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
        wasm.__wbindgen_export3(deferred3_0, deferred3_1, 1);
    }
}

/**
 * 巡检文本 DSL：成功信封返回 `{bindings,error,error_offset,line,column}`。
 *
 * 解析失败时错误保留在成功值内部，而不是整体变成 error 信封，这样前端右侧速查栏
 * 可以同时展示已扫描到的绑定；只有 `questions_json` 本身非法才返回边界错误。
 * @param {string} source
 * @param {string} questions_json
 * @returns {string}
 */
export function inspect_logic_text(source, questions_json) {
    let deferred3_0;
    let deferred3_1;
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16) >>> 0;
        let ptr0, len0, ptr1, len1;
        try {
            ptr0 = passStringToWasm0(source, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            len0 = WASM_VECTOR_LEN;
            ptr1 = passStringToWasm0(questions_json, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            len1 = WASM_VECTOR_LEN;
        } catch (e) {
            if (ptr1) { wasm.__wbindgen_export3(ptr1, len1 * 1, 1); }
            if (ptr0) { wasm.__wbindgen_export3(ptr0, len0 * 1, 1); }
            throw e;
        }
        wasm.inspect_logic_text(retptr, ptr0, len0, ptr1, len1);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        deferred3_0 = r0;
        deferred3_1 = r1;
        return getStringFromWasm0(r0, r1);
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
        wasm.__wbindgen_export3(deferred3_0, deferred3_1, 1);
    }
}

/**
 * 解析文本 DSL；失败信封额外携带 `line` / `column`。
 * @param {string} source
 * @param {string} questions_json
 * @returns {string}
 */
export function parse_logic_text(source, questions_json) {
    let deferred3_0;
    let deferred3_1;
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16) >>> 0;
        let ptr0, len0, ptr1, len1;
        try {
            ptr0 = passStringToWasm0(source, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            len0 = WASM_VECTOR_LEN;
            ptr1 = passStringToWasm0(questions_json, wasm.__wbindgen_export, wasm.__wbindgen_export2);
            len1 = WASM_VECTOR_LEN;
        } catch (e) {
            if (ptr1) { wasm.__wbindgen_export3(ptr1, len1 * 1, 1); }
            if (ptr0) { wasm.__wbindgen_export3(ptr0, len0 * 1, 1); }
            throw e;
        }
        wasm.parse_logic_text(retptr, ptr0, len0, ptr1, len1);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        deferred3_0 = r0;
        deferred3_1 = r1;
        return getStringFromWasm0(r0, r1);
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
        wasm.__wbindgen_export3(deferred3_0, deferred3_1, 1);
    }
}
export function __wbg___wbindgen_throw_4c88d8bf264fd984(arg0, arg1) {
    throw new Error(getStringFromWasm0(arg0, arg1));
}
export function __wbindgen_generic_0000000000000001(arg0, arg1) {
    // Cast intrinsic for `Ref(String) -> Externref`.
    const ret = getStringFromWasm0(arg0, arg1);
    return ret;
}
export function __wbindgen_init_externref_table() {
    const table = wasm.__wbindgen_externrefs;
    table.set(0, undefined);
    table.set(1024, undefined);
    table.set(1025, null);
    table.set(1026, true);
    table.set(1027, false);
}
const FlowSessionFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_flowsession_free(ptr, 1));

let cachedDataViewMemory0 = null;
function getDataViewMemory0() {
    if (cachedDataViewMemory0 === null || cachedDataViewMemory0.buffer.detached === true || (cachedDataViewMemory0.buffer.detached === undefined && cachedDataViewMemory0.buffer !== wasm.memory.buffer)) {
        cachedDataViewMemory0 = new DataView(wasm.memory.buffer);
    }
    return cachedDataViewMemory0;
}

function getStringFromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return decodeText(getUint8ArrayMemory0().subarray(ptr, ptr + len));
}

let cachedUint8ArrayMemory0 = null;
function getUint8ArrayMemory0() {
    if (cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0) {
        cachedUint8ArrayMemory0 = new Uint8Array(wasm.memory.buffer);
    }
    return cachedUint8ArrayMemory0;
}

function passStringToWasm0(arg, malloc, realloc) {
    if (realloc === undefined) {
        const buf = cachedTextEncoder.encode(arg);
        const ptr = malloc(buf.length, 1) >>> 0;
        getUint8ArrayMemory0().subarray(ptr, ptr + buf.length).set(buf);
        WASM_VECTOR_LEN = buf.length;
        return ptr;
    }

    let len = arg.length;
    let ptr = malloc(len, 1) >>> 0;

    const mem = getUint8ArrayMemory0();

    let offset = 0;

    for (; offset < len; offset++) {
        const code = arg.charCodeAt(offset);
        if (code > 0x7F) break;
        mem[ptr + offset] = code;
    }
    if (offset !== len) {
        if (offset !== 0) {
            arg = arg.slice(offset);
        }
        ptr = realloc(ptr, len, len = offset + arg.length * 3, 1) >>> 0;
        const view = getUint8ArrayMemory0().subarray(ptr + offset, ptr + len);
        const ret = cachedTextEncoder.encodeInto(arg, view);

        offset += ret.written;
        ptr = realloc(ptr, len, offset, 1) >>> 0;
    }

    WASM_VECTOR_LEN = offset;
    return ptr;
}

function takeFromExternrefTable0(idx) {
    const value = wasm.__wbindgen_externrefs.get(idx);
    wasm.__wbindgen_export4(idx);
    return value;
}

let cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true });
cachedTextDecoder.decode();
const MAX_SAFARI_DECODE_BYTES = 2146435072;
let numBytesDecoded = 0;
function decodeText(bytes) {
    numBytesDecoded += bytes.length;
    if (numBytesDecoded >= MAX_SAFARI_DECODE_BYTES) {
        cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true });
        cachedTextDecoder.decode();
        numBytesDecoded = bytes.length;
    }
    return cachedTextDecoder.decode(bytes);
}

const cachedTextEncoder = new TextEncoder();

if (!('encodeInto' in cachedTextEncoder)) {
    cachedTextEncoder.encodeInto = function (arg, view) {
        const buf = cachedTextEncoder.encode(arg);
        view.set(buf);
        return {
            read: arg.length,
            written: buf.length
        };
    };
}

let WASM_VECTOR_LEN = 0;


let wasm;
export function __wbg_set_wasm(val) {
    wasm = val;
}
