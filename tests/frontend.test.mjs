// 前端逻辑测试：把 static/index.html 里的 <script> 抠出来放进 vm 沙箱跑，
// 用最小 DOM/localStorage 桩覆盖转义、XSS 回归、历史记录与配色映射。
// 运行：node --test tests/frontend.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import vm from 'node:vm';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const html = readFileSync(join(root, 'static', 'index.html'), 'utf8');
const script = html.match(/<script>([\s\S]*?)<\/script>/)[1];

// 最小元素桩：记录 classList 状态，够页面脚本初始化与被测函数使用
function makeEl() {
  const el = {
    value: '',
    textContent: '',
    innerHTML: '',
    hidden: false,
    placeholder: '',
    classList: {
      _set: {},
      add(...cs) { for (const c of cs) this._set[c] = true; },
      remove(...cs) { for (const c of cs) delete this._set[c]; },
      toggle(c, force) {
        if (force === undefined) this._set[c] = !this._set[c];
        else if (force) this._set[c] = true;
        else delete this._set[c];
      },
      contains(c) { return !!this._set[c]; },
    },
    style: {},
    focus() {},
    select() {},
    click() {},
    remove() {},
    insertAdjacentHTML(_pos, html) { el.lastInserted = html; },
    querySelector: () => makeEl(),
    closest: () => null,
  };
  return el;
}

const els = {};
const sandbox = {
  document: {
    getElementById: (id) => (els[id] ??= makeEl()),
    querySelectorAll: () => [],
    addEventListener: () => {},
  },
  localStorage: { getItem: () => null, setItem: () => {}, removeItem: () => {} },
  fetch: async () => { throw new Error('测试中不应真的发请求'); },
  AbortController: class { abort() {} },
  TextEncoder,
  console,
  Date,
};
vm.createContext(sandbox);
vm.runInContext(script, sandbox, { filename: 'index.html<script>' });

// 顶层 let/const 活在沙箱的词法作用域里，取值求值需再次进沙箱；
// 返回值经 JSON 往返拉回主 realm，避免跨 realm 原型差异干扰 deepStrictEqual
const ev = (expr) => {
  const v = vm.runInContext(expr, sandbox);
  return v === undefined ? undefined : JSON.parse(JSON.stringify(v));
};

test('页面脚本在桩环境下可完整初始化', () => {
  assert.ok(els['headersContainer'], 'headersContainer 应已就绪');
  assert.ok(ev(`document.getElementById('historyList').innerHTML`).length > 0);
});

test('esc 转义 & < >', () => {
  assert.equal(ev(`esc('<img src=x onerror=alert(1)>')`), '&lt;img src=x onerror=alert(1)&gt;');
  assert.equal(ev(`esc('a&b')`), 'a&amp;b');
});

test('escAttr 额外转义引号，防属性逃逸', () => {
  assert.equal(ev(`escAttr('" onmouseover="pwn')`), '&quot; onmouseover=&quot;pwn');
});

test('byteLen 按 UTF-8 字节计，中文不再虚胖', () => {
  assert.equal(ev(`byteLen('ab')`), 2);
  assert.equal(ev(`byteLen('中')`), 3);
  // 5 个汉字 ×3 字节 + 2 个半角 = 17
  assert.equal(ev(`byteLen('中文字符串!!')`), 17);
});

test('fmtSize 三档', () => {
  assert.equal(ev(`fmtSize(500)`), '500 字节');
  assert.equal(ev(`fmtSize(2048)`), '2.0KB');
  assert.equal(ev(`fmtSize(3*1024*1024)`), '3.00MB');
});

test('方法配色映射完整，未知方法兜底 m-other', () => {
  assert.equal(ev(`mClass('GET')`), 'm-get');
  assert.equal(ev(`mClass('POST')`), 'm-post');
  assert.equal(ev(`mClass('PUT')`), 'm-put');
  assert.equal(ev(`mClass('DELETE')`), 'm-delete');
  assert.equal(ev(`mClass('PATCH')`), 'm-patch');
  assert.equal(ev(`mClass('HEAD')`), 'm-head');
  assert.equal(ev(`mClass('delete')`), 'm-delete', '大小写不敏感');
  assert.equal(ev(`mClass('TRACE')`), 'm-other');
  assert.equal(ev(`mClass('')`), 'm-other');
});

test('状态码配色 2xx/3xx/4xx/5xx/其他', () => {
  assert.equal(ev(`sClass(200)`), 's2');
  assert.equal(ev(`sClass(299)`), 's2');
  assert.equal(ev(`sClass(301)`), 's3');
  assert.equal(ev(`sClass(404)`), 's4');
  assert.equal(ev(`sClass(500)`), 's5');
  assert.equal(ev(`sClass(0)`), 's0');
});

test('syntaxHighlight 是 XSS 防线：token 里的标签必须被转义', () => {
  const out = ev(
    `syntaxHighlight(JSON.stringify({a:'<script>alert(1)<\\/script>', b:'<img src=x onerror=y>'}))`
  );
  assert.ok(!out.includes('<script>'), '不允许裸 <script>');
  assert.ok(!out.includes('<img'), '不允许裸 <img>');
  assert.ok(out.includes('&lt;script&gt;'), '应包含转义后的标签');
});

test('addHistory 过滤敏感请求头，重名保留', () => {
  ev(`addHistory('POST', 'http://x/', [
    ['authorization', 'Bearer t'],
    ['Cookie', 'a=b'],
    ['x-api-key', 'k'],
    ['x-ok', '1'],
    ['x-ok', '2'],
  ], 'body', 200, 5)`);
  assert.deepEqual(ev(`history[0].headers`), [['x-ok', '1'], ['x-ok', '2']]);
});

test('addHistory 超 64KB 的请求体不入库并打标', () => {
  ev(`addHistory('POST','http://x/',[], 'x'.repeat(64*1024+1), 200, 1)`);
  assert.equal(ev(`history[0].body`), null);
  assert.equal(ev(`history[0].bodyOmitted`), true);
  // 正常大小的照存
  ev(`addHistory('POST','http://x/',[], 'small', 200, 1)`);
  assert.equal(ev(`history[0].body`), 'small');
  assert.equal(ev(`history[0].bodyOmitted`), false);
});

test('历史上限 20 条，新的留下旧的滚出', () => {
  ev(`history.length = 0; for (let i = 0; i < 25; i++) addHistory('GET', 'u'+i, [], null, 200, 1)`);
  assert.equal(ev(`history.length`), 20);
  assert.equal(ev(`history[0].url`), 'u24');
  assert.equal(ev(`history[19].url`), 'u5');
});

test('renderHistory 对恶意 method/url 一律转义', () => {
  ev(`history.length = 0`);
  ev(`addHistory('<img src=x onerror=y>', 'http://x/"onmouseover="pwn', [], null, 200, 1)`);
  const html = ev(`document.getElementById('historyList').innerHTML`);
  assert.ok(!html.includes('<img'), 'method 不允许裸标签');
  assert.ok(!html.includes('onmouseover="pwn"'), 'url 不允许属性逃逸');
  assert.ok(html.includes('&lt;img'), '应包含转义后的内容');
});

test('switchTab 只激活一侧', () => {
  ev(`switchTab('req', 'body')`);
  assert.ok(els['req-body-panel'].classList.contains('active'), 'body 面板应激活');
  assert.ok(!els['req-headers-panel'].classList.contains('active'), 'headers 面板应隐藏');
  assert.ok(els['req-tab-body'].classList.contains('active'));
  ev(`switchTab('req', 'headers')`);
  assert.ok(els['req-headers-panel'].classList.contains('active'));
  assert.ok(!els['req-body-panel'].classList.contains('active'));
});

test('syncMethodColor 跟随下拉框取值', () => {
  els['method'].value = 'DELETE';
  ev(`syncMethodColor()`);
  assert.ok(els['method'].classList.contains('m-delete'));
  els['method'].value = 'GET';
  ev(`syncMethodColor()`);
  assert.ok(els['method'].classList.contains('m-get'));
  assert.ok(!els['method'].classList.contains('m-delete'));
});
