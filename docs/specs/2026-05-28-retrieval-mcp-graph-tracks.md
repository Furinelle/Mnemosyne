# Mnemosyne v0.2 — 三轨升级 spec

**日期:** 2026-05-28
**基线 commit:** `dce605a`
**目标交付:** 通过 Codex 一次性目标追求模式执行
**预估代码改动:** ~2000-3000 行(含测试)

---

## 1. 背景与目标

Mnemosyne v0.1(当前 `master`)已具备:文件即接口的 markdown 存储、BM25 in-memory 检索、SQLite FTS5 持久索引(带 WAL 并发保护、增量 mtime 同步)、Claude Code 4 hooks、Codex 双向通道、强度衰减生命周期、token-budgeted 注入。

参考同类项目对比(TencentDB-Agent-Memory、mem0、letta、Zep)后,v0.1 的三大短板:
1. **CJK 召回弱**——`[\w一-鿿]+` 把整串中文当一个 token,"认证" 查不到 "调试认证失败"
2. **只能给 Claude+Codex 用**——MCP 协议未支持,Cursor/Cline/Continue/Windsurf 等无法接入
3. **链接是死数据**——`link` 命令建了双向边,但搜索、排序、扩展都没用上;关系无类型,无法语义区分

v0.2 同时启动三条**互相独立**的升级轨道,在同一个 PR/commit 周期内交付:

- **轨道 1 — 检索质量大跃迁:** CJK bigram + 可插拔 embedder + RRF + 跨编码器 reranker + 评测套件
- **轨道 2 — 通用 MCP 服务化:** mnemosyne 包装为 MCP server,任何 MCP 客户端可接
- **轨道 3 — 知识图谱关系类型化:** 预定义关系语义 + graph CLI + 关系加权 link expansion

**唯一耦合点:** 轨道 3 的关系权重表给轨道 1 的 link expansion 取代固定 0.5 衰减。

## 2. 范围与非目标

### 2.1 本轮做(In scope)

| | 轨道 1 | 轨道 2 | 轨道 3 |
|---|---|---|---|
| 新增模块 | `tokenizer.py`、`embedding/`、`rerank/`、`fusion.py`、`eval/` | `mcp/server.py` | `relations.py`、`graph.py` |
| 修改模块 | `index.py`、`search.py`、`cli.py`、`store.py` | `cli.py` | `cli.py` |
| 新增 CLI | `embed-backfill`、`eval run`、`eval compare` | `mcp serve` | `graph` |
| 新增配置段 | `[embedding]`、`[rerank]`、`[fusion]` | `[mcp]` | `[relations]` |
| 新增可选依赖 | `mnemosyne[vector]` (onnxruntime,numpy), `mnemosyne[rerank]` | `mnemosyne[mcp]` (mcp SDK) | 无 |

### 2.2 本轮不做(Out of scope,留下一轮)

- LLM backend 抽象层 / DeepSeek-OpenAI 兼容客户端
- Stop hook 自动总结 handoff
- L0-L3 抽象金字塔(TencentDB 风格)
- 智能去重 / maintain --consolidate
- 多机同步 / export-import bundle
- Web/TUI 浏览器
- 隐私脱敏 / PII 检测

### 2.3 设计原则(贯穿三轨)

1. **可选依赖、默认关闭:** 基础 `pip install mnemosyne` 仍只要 `portalocker`。所有新依赖通过 extras 进入。
2. **行为回归保护:** v0.1 的 BM25-only 路径必须在 v0.2 默认配置下产生**等价**结果(除 CJK 分词必然改变 token 切分外)。
3. **可观测可验证:** eval harness 给出可复现的数字,所有性能/质量声明必须用 `mnemosyne eval` 跑出来证明。
4. **并发安全继承:** SQLite 所有写路径走 `_connect()` (WAL + busy_timeout),向量/embedding 列的写入也必须遵守。
5. **Hook 永不阻塞:** `hook_safe()` 装饰器规则不变;新增的 LLM/向量调用必须可超时、可降级。

---

## 3. 轨道 1 — 检索质量大跃迁

### 3.1 模块布局

```
mnemosyne/
├── tokenizer.py            (新)
├── embedding/              (新)
│   ├── __init__.py         (Embedder Protocol + get_embedder factory)
│   ├── base.py             (Embedder abstract, NoneEmbedder fallback)
│   ├── onnx.py             (LocalONNXEmbedder, lazy import onnxruntime)
│   └── openai.py           (OpenAICompatEmbedder,lazy import httpx)
├── rerank/                 (新)
│   ├── __init__.py         (Reranker Protocol + get_reranker factory)
│   ├── base.py             (NoneReranker pass-through)
│   └── cross_encoder.py    (CrossEncoderReranker, lazy import onnxruntime)
├── fusion.py               (新,RRF + link expansion 调度)
├── eval/                   (新)
│   ├── corpus.py           (load/save synthetic eval corpus)
│   ├── metrics.py          (recall@k, MRR, latency)
│   ├── default_corpus.jsonl(50 条基线)
│   └── __main__.py         (`python -m mnemosyne.eval ...`)
├── index.py                (扩展)
├── search.py               (扩展,tokenize 代理到 tokenizer.py)
└── cli.py                  (扩展)
```

### 3.2 Tokenizer(CJK bigram)

**问题:** 当前 `TOKEN_RE = r"[\w一-鿿]+"` 把 CJK 连续字符当作单 token。

**方案:** 在 `mnemosyne/tokenizer.py` 实现:

```python
def tokenize(text: str) -> list[str]:
    """Split into bigrams for CJK runs + word tokens for non-CJK."""
    text = text.lower()
    tokens: list[str] = []
    for match in re.finditer(r"[a-z0-9_]+|[一-鿿]+", text):
        chunk = match.group(0)
        if _is_cjk(chunk[0]):
            if len(chunk) == 1:
                tokens.append(chunk)
            else:
                for i in range(len(chunk) - 1):
                    tokens.append(chunk[i:i+2])
        else:
            tokens.append(chunk)
    return tokens
```

- 单字 CJK 保留为 1-gram(避免漏)
- 多字 CJK 拆 bigram("调试认证" → `["调试","试认","认证"]`)
- 字母数字下划线连续段保持原样
- 完全语言无关(中/日/韩同样生效)

**FTS5 端:** `index.py` 中 `CREATE VIRTUAL TABLE memories_fts USING fts5(..., tokenize="trigram")` —— SQLite 3.34+ 内置 trigram tokenizer,自动对 CJK 友好。bigram(Python)+ trigram(FTS5)略不一致但都解决 CJK 召回问题,搜索时 Python 端把 bigram token 用 OR 拼起来给 FTS5 即可(已有 `_query_expression` 逻辑)。

**迁移:** `search.py` 中 `tokenize`/`TOKEN_RE`/`memory_search_text` 改为从 `mnemosyne.tokenizer` 导入,保持公开 API 兼容。

### 3.3 Embedder 抽象

```python
# mnemosyne/embedding/base.py
class Embedder(Protocol):
    dimensions: int
    model_id: str  # 用于在 index 标识 embedding 兼容性

    def embed(self, texts: list[str]) -> list[list[float]]: ...
    def embed_one(self, text: str) -> list[float]: ...

class NoneEmbedder:
    """Returns None vectors; used when embedding.enabled=false."""
    dimensions = 0
    model_id = "none"
    def embed(self, texts): return [None] * len(texts)
    def embed_one(self, text): return None
```

**工厂:**
```python
def get_embedder(config: dict) -> Embedder:
    if not config.get("embedding", {}).get("enabled"):
        return NoneEmbedder()
    backend = config["embedding"]["backend"]
    if backend == "onnx":
        from mnemosyne.embedding.onnx import LocalONNXEmbedder
        return LocalONNXEmbedder(config["embedding"])
    if backend == "openai":
        from mnemosyne.embedding.openai import OpenAICompatEmbedder
        return OpenAICompatEmbedder(config["embedding"])
    raise ValueError(f"unknown embedding backend: {backend}")
```

**ONNX backend:**
- 默认模型:`BAAI/bge-small-zh-v1.5`(384d, ~50MB)
- 模型路径:`config.embedding.onnx_path`(留空则下载到 `~/.cache/mnemosyne/models/`)
- 推理:`onnxruntime.InferenceSession`,batch_size 32,L2 normalize 输出
- 失败行为:首次 init 下载失败 → fallback 到 `NoneEmbedder` 并 stderr warn(不让 hook 崩)

**OpenAI 兼容 backend:**
- 字段:`api_base`、`api_key_env`、`model`、`dimensions`
- 任何 OpenAI 兼容服务(原生 OpenAI、Voyage、Cohere 兼容代理、本地 ollama)都能用
- 调用:`POST {api_base}/embeddings` body `{"model": ..., "input": [...]}`
- httpx 超时 10s,重试 1 次

### 3.4 向量存储

**不引入 sqlite-vec 扩展**——macOS Homebrew Python 默认禁 `enable_load_extension`,会破开箱即用。

改在 `memories_meta` 加列:

```sql
ALTER TABLE memories_meta ADD COLUMN embedding BLOB;
ALTER TABLE memories_meta ADD COLUMN embedding_model TEXT NOT NULL DEFAULT '';
ALTER TABLE memories_meta ADD COLUMN embedding_dim INTEGER NOT NULL DEFAULT 0;
ALTER TABLE memories_meta ADD COLUMN embedding_mtime REAL NOT NULL DEFAULT 0;
```

- BLOB 存 float16 数组(`numpy.float16.tobytes()`),384d × 2 byte = 768 byte/条
- `embedding_model` 记录写入时的 model_id;查询时若当前 embedder.model_id ≠ 存储值,视为无 embedding(需要 `embed-backfill` 重算)
- `INDEX_VERSION = 2`,启动时检测,< 2 则 ALTER TABLE 增列

**Cosine 计算:** 纯 stdlib `array.array("f", ...)` + python loop,对 384d × 1 万条耗时 < 30ms。可选 `numpy` 加速(若安装则用)。

### 3.5 Reranker 抽象

类似 embedder:

```python
class Reranker(Protocol):
    model_id: str
    def rerank(self, query: str, docs: list[str]) -> list[float]: ...

class NoneReranker:
    model_id = "none"
    def rerank(self, query, docs): return [0.0] * len(docs)
```

**CrossEncoderReranker:**
- 默认模型:`BAAI/bge-reranker-base`(~110MB ONNX)
- 输入:`(query, doc)` pair,输出 logit
- 仅对 RRF 后的 top-N 跑(默认 top_n=5),避免对全 corpus 推理
- 失败行为:同 embedder,fallback NoneReranker + warn

### 3.6 检索流水线

`mnemosyne/fusion.py` 总编排:

```
search(query, limit, type_filter, scope, include_archive)
 │
 ├─ tokenize(query) → bigram tokens
 │
 ├─ BM25 lane (FTS5 or in-memory fallback)
 │    └─ top-K_BM25 候选 (K_BM25 = limit*10, 默认 50)
 │
 ├─ Vector lane (仅 embedder.enabled)
 │    ├─ embedder.embed_one(query) → q_vec
 │    ├─ 全 store 扫 memories_meta,拉所有 embedding BLOB
 │    ├─ cosine(q_vec, e) for each
 │    └─ top-K_VEC 候选 (K_VEC = limit*10)
 │
 ├─ RRF fusion (k=60)
 │    score(doc) = Σ_lane 1 / (k + rank_lane(doc))
 │    每条 lane 内按本 lane 分数排名
 │    输出 top-(limit*3) 候选
 │
 ├─ Link expansion (仅 fusion.link_expansion)
 │    对每个候选 d,枚举 d.links:
 │      target = lookup(link.id)
 │      weight = relations.weight(link.rel)  ← 轨道 3 注入
 │      target.score += d.score * weight * depth_decay
 │    合并候选集
 │
 ├─ Rerank (仅 rerank.enabled)
 │    取当前 top-(top_n*2) 喂 cross-encoder
 │    用 reranker 分数替换 (或加权平均)
 │
 └─ 截 top-limit 返回,带 score_breakdown {bm25, vec, link_boost, rerank}
```

**回归保护:** embedder/reranker/link_expansion 全关时,流水线等价于"BM25 → top-limit"。CJK bigram 之外的差异为 0(行为完全不变)。

### 3.7 Eval harness

`mnemosyne/eval/corpus.py`:

```python
@dataclass
class EvalItem:
    query: str
    expected_ids: list[str]    # ground-truth memory IDs (顺序无关)
    paraphrase_of: str         # 该 query 是哪条记忆的同义改写
    notes: str = ""

def load_corpus(path: Path) -> list[EvalItem]: ...
def save_corpus(path: Path, items: list[EvalItem]) -> None: ...
```

`mnemosyne/eval/metrics.py`:

```python
def recall_at_k(results: list[str], expected: list[str], k: int) -> float: ...
def mrr(results: list[str], expected: list[str]) -> float: ...
def latency_percentiles(durations: list[float]) -> dict: ...
```

`mnemosyne/eval/default_corpus.jsonl`:
- **基线 50 条**,覆盖 5 种记忆类型各 10 条
- 每条对应一条预置 memory(在 `eval/seed_memories.jsonl` 里)
- query 风格混合:同义改写、否定、换语言(中英互译)、模糊措辞、关键词丢失

CLI:

```bash
mnemosyne eval run [--corpus FILE] [--scope project]
  # 输出:
  # backend: bm25-only       recall@5=0.42  MRR=0.31  p50=8ms  p99=22ms
  # backend: bm25+bigram     recall@5=0.61  MRR=0.45  p50=9ms  p99=24ms
  # ...

mnemosyne eval compare --baseline config_a.toml --variant config_b.toml [--corpus FILE]
  # A/B 对比同一语料,出 delta 表
```

`eval run` 默认按当前 `config.toml` 跑;`compare` 用两份临时 config 跑两次出对比。

### 3.8 配置(`config.toml`)

```toml
[embedding]
enabled = false
backend = "onnx"                          # "onnx" | "openai"
model = "BAAI/bge-small-zh-v1.5"
onnx_path = ""                            # 空则下载到 ~/.cache/mnemosyne/models/
api_base = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"
dimensions = 384
batch_size = 32

[rerank]
enabled = false
backend = "cross_encoder"                 # "cross_encoder" | "none"
model = "BAAI/bge-reranker-base"
onnx_path = ""
top_n = 5

[fusion]
rrf_k = 60
link_expansion = true
link_expansion_decay_fallback = 0.5       # 关系无类型时的衰减
link_expansion_max_hops = 1
bm25_pool_size = 50                       # FTS top-K 候选池
vec_pool_size = 50                        # vector top-K 候选池
```

### 3.9 CLI 新增

| 命令 | 说明 |
|---|---|
| `mnemosyne embed-backfill [--scope all]` | 给现有记忆补算 embedding(切换 embedder 模型后必跑) |
| `mnemosyne eval run [--corpus FILE]` | 单次评测 |
| `mnemosyne eval compare --baseline A.toml --variant B.toml [--corpus FILE]` | A/B 对比 |

`reindex` 命令扩展:若 `embedding.enabled`,顺带重算所有 embedding。

### 3.10 测试

- `tests/test_tokenizer.py`:CJK bigram 正确性、中英混、emoji、CJK 标点、空串
- `tests/test_embedding.py`:NoneEmbedder fallback、mock backend 接口契约
- `tests/test_fusion.py`:RRF 数学(对照标准实现)、link expansion 衰减、边界(空候选、单候选)
- `tests/test_eval.py`:metrics 正确性、eval CLI 端到端
- `tests/test_search_v2.py`:hybrid 路径(mock embedder 用 hash→向量)、回归 BM25-only 路径
- `tests/test_concurrency.py`:embedder 启用下两 process 并发 write+search 不冲突

---

## 4. 轨道 2 — 通用 MCP 服务化

### 4.1 模块布局

```
mnemosyne/mcp/
├── __init__.py
└── server.py            (实现 MCP server,stdio + optional sse)

templates/mcp_clients/   (新目录)
├── cursor.json
├── cline.json
├── continue.json
└── windsurf.json        (各客户端的接入片段)
```

### 4.2 可选依赖

`pip install mnemosyne[mcp]` 进 `mcp >= 0.4.0` (Anthropic 官方 Python SDK)。

### 4.3 暴露的 tools

| MCP tool | 对应 CLI | 输入 | 输出 |
|---|---|---|---|
| `mnemosyne_search` | `search` | `{query, limit?, type?, scope?, include_archive?}` | top-K 结果 JSON |
| `mnemosyne_write` | `write --force` | `{type, importance, title?, content, tags?, scope?, source?}` | `{id}` |
| `mnemosyne_read_core` | `read` | `{scope?}` | core memory markdown |
| `mnemosyne_show` | `show` | `{id}` | 完整 frontmatter+body |
| `mnemosyne_link` | `link --rel` | `{id1, id2, rel?}` | `{ok}` |
| `mnemosyne_graph` | `graph` | `{id, depth?, format?}` | mermaid 字符串或 json |
| `mnemosyne_maintain` | `maintain --dry-run` | `{scope?, dry_run?}` | summary JSON |
| `mnemosyne_codex_prep` | `codex-prep` | `{task, limit?}` | prompt prefix |

**安全:** 所有 tool 都内部走标准 CLI 实现(`cli.cmd_*` 抽出可调函数),无 shell 转义、无任意路径访问。`write` 默认 `--force`(MCP 调用方是 Agent,人机交互去重没意义)。

### 4.4 传输

- **默认 stdio**:`mnemosyne mcp serve` 启动后从 stdin 读 JSON-RPC,stdout 回。这是 Cursor/Cline/Continue 等标准接入方式。
- **可选 SSE**:`[mcp.sse] enabled=true port=3700`,适合远程或调试。

### 4.5 配置

```toml
[mcp]
expose_global = true                     # 暴露全局 store 的 tools
expose_project = true                    # 暴露项目 store 的 tools(server 启动目录决定)
default_search_limit = 5

[mcp.sse]
enabled = false
port = 3700
host = "127.0.0.1"
```

### 4.6 CLI

```bash
mnemosyne mcp serve              # stdio 服务
mnemosyne mcp serve --sse        # 启 SSE
```

### 4.7 客户端接入模板

`templates/mcp_clients/` 下放各客户端 settings 片段:

`cursor.json` (示例):
```json
{
  "mcpServers": {
    "mnemosyne": {
      "command": "mnemosyne",
      "args": ["mcp", "serve"],
      "env": {"MNEMOSYNE_HOME": "~/.mnemosyne"}
    }
  }
}
```

各客户端片段 README 里也要列出怎么用。

### 4.8 测试

- `tests/test_mcp_server.py`:
  - `tools/list` 返回的 schema 与文档一致
  - `tools/call mnemosyne_search` 端到端(stdio 启 server 子进程,发 JSON-RPC,验返回)
  - `tools/call mnemosyne_write` 写入后能 search 到
  - 错误处理:无效 tool name、缺字段、JSON 解析失败均返标准 MCP error

---

## 5. 轨道 3 — 知识图谱关系类型化

### 5.1 模块布局

```
mnemosyne/
├── relations.py       (新,预定义关系表 + 校验)
├── graph.py           (新,BFS + Mermaid/ASCII/JSON 渲染)
└── cli.py             (扩,link 加校验, 新 graph 命令)
```

### 5.2 预定义关系

```python
# mnemosyne/relations.py
PREDEFINED = {
    "caused_by":   RelationSpec(reverse="causes",       weight=0.6, symmetric=False),
    "refines":     RelationSpec(reverse="refined_by",   weight=0.7, symmetric=False),
    "supersedes":  RelationSpec(reverse="superseded_by",weight=0.3, symmetric=False, demote_target=True),
    "contradicts": RelationSpec(reverse="contradicts",  weight=0.5, symmetric=True,  warn=True),
    "related":     RelationSpec(reverse="related",      weight=0.5, symmetric=True),
}

def weight(rel: str, config_override: dict | None = None) -> float:
    """Return expansion weight for a relation, with config override support."""
    overrides = (config_override or {})
    if rel in overrides:
        return float(overrides[rel])
    spec = PREDEFINED.get(rel)
    return spec.weight if spec else 0.5  # 未知关系按 0.5 衰减(向后兼容)

def reverse(rel: str) -> str | None: ...
def is_symmetric(rel: str) -> bool: ...
def is_demoting(rel: str) -> bool: ...
def warns(rel: str) -> bool: ...
```

**`demote_target`:** `supersedes` 关系下,被 supersede 的旧记忆在 maintain 时强度多衰减一份(预留接口,本轮可不实现,留 TODO)。

### 5.3 `link` 命令变化

- 不带 `--allow-custom`:`--rel` 必须在 `PREDEFINED.keys()` 内,否则报错列出可选值
- 带 `--allow-custom`:任意字符串允许
- 对非对称关系自动写反向:`link A B --rel supersedes` → A.links 加 `{id: B, rel: supersedes}` 同时 B.links 加 `{id: A, rel: superseded_by}`
- 对称关系(`related`、`contradicts`)两边写相同 rel

### 5.4 `graph` 命令

```bash
mnemosyne graph MEMORY_ID [--depth N] [--format mermaid|ascii|json]
```

- BFS 从 MEMORY_ID 出发,展开 depth 跳
- 环检测(visited set)
- 输出:
  - `mermaid`(默认):
    ```mermaid
    graph LR
      pitfall-A[Redis 漏池]
      arch-B[改用连接池]
      pitfall-A -- caused_by --> arch-B
    ```
  - `ascii`:树状缩进
  - `json`:`{nodes: [...], edges: [...]}`,给 MCP `mnemosyne_graph` 和未来前端用

### 5.5 与轨道 1 link expansion 集成

`mnemosyne/fusion.py` 的 link expansion 阶段调用 `relations.weight(link.rel, fusion_config.get("relation_weight_override"))` 而不是固定 0.5。`contradicts` 命中时在结果 `score_breakdown` 里加 `"contradicts_with": [id, ...]`,前端可显示警告。

### 5.6 配置

```toml
[relations]
# Override 默认权重(可选)
# weight_caused_by = 0.7
# weight_contradicts = 0.4
allow_custom = false              # CLI link 默认是否允许非预定义关系
```

### 5.7 测试

- `tests/test_relations.py`:权重查询、反向关系、对称判断
- `tests/test_graph.py`:BFS 正确性、环检测、Mermaid 输出 snapshot、ASCII 树
- `tests/test_link_typed.py`:link CLI 校验、自动反向写入、对称关系
- 集成:`tests/test_fusion.py` 加 typed expansion 用例(refines 0.7 vs related 0.5)

---

## 6. 跨轨集成点

1. **轨道 3 → 轨道 1:** `fusion.py` 的 link expansion 调用 `relations.weight(rel, override)`
2. **轨道 2 → 轨道 1+3:** MCP server 的 `mnemosyne_search` 直接复用 `fusion.search`,`mnemosyne_graph` 直接复用 `graph.bfs`
3. **轨道 2 ↔ 轨道 3:** MCP `mnemosyne_link` 自动应用关系校验和反向写入

---

## 7. 配置全景(合并)

```toml
# 既有(v0.1)
[thresholds]
[memory]
[injection]
[search]

# v0.2 新增
[embedding]
[rerank]
[fusion]
[relations]
[mcp]
[mcp.sse]
```

`store.py` 中 `DEFAULT_CONFIG` 和 `DEFAULT_CONFIG_TOML` 同步更新,`load_config` 自动 deep-merge。

---

## 8. CLI 全景

```bash
# v0.1
init / read / write / search / show / link / maintain
codex-prep / codex-ingest / reindex / doctor

# v0.2 新增
embed-backfill [--scope]
eval run [--corpus]
eval compare --baseline --variant [--corpus]
graph ID [--depth] [--format]
mcp serve [--sse]
```

---

## 9. 测试矩阵

| 类别 | 文件 | 用例 |
|---|---|---|
| 单元(v0.1 回归) | 全套已有测试 | 必须全过 |
| 单元(轨道1) | test_tokenizer/embedding/fusion/eval | 见 3.10 |
| 单元(轨道2) | test_mcp_server | 见 4.8 |
| 单元(轨道3) | test_relations/graph/link_typed | 见 5.7 |
| 集成 | test_search_v2 | hybrid 流水线、回归 BM25-only |
| 集成 | test_eval_e2e | 跑 default_corpus 出指标 |
| 集成 | test_mcp_e2e | 子进程启 server 实跑 JSON-RPC |
| 并发 | test_concurrency_v2 | embedder/index 并发写不死锁 |

CI(若需要):新增 `pytest tests/` 在 `.github/workflows/` 或 README 文档化。

---

## 10. 向后兼容 / 迁移

- **索引迁移:** `INDEX_VERSION` 1→2,启动时 ALTER TABLE 加列,不 drop
- **配置兼容:** v0.1 `config.toml` 无新段,代码默认值兜底,**不需要用户手动改**
- **行为兼容:** 所有新功能默认 `enabled=false`;CJK tokenizer 改变是预期改进(无 query 输出格式变化)
- **依赖兼容:** 基础包不引入新依赖;新功能走 extras
- **CLI 兼容:** 既有命令签名不变

---

## 11. 验收标准(Definition of Done)

1. 全套测试通过(v0.1 既有 + 本 spec 列出的所有新测试),`python -m unittest discover -s tests` 退出 0
2. `python -m py_compile` 所有新增/修改 `.py` 文件无错
3. `mnemosyne eval run --corpus mnemosyne/eval/default_corpus.jsonl` 至少产生一份基线指标(BM25-only)
4. `mnemosyne mcp serve` 启动后能响应 `tools/list` 和 `tools/call mnemosyne_search`
5. `mnemosyne graph <id> --format mermaid` 对有 links 的记忆输出有效 Mermaid
6. `mnemosyne doctor` 报告新增 embedder/reranker/mcp 状态(可用/未配置/不可用)
7. README 增补三轨的简介小节
8. 基础安装 `pip install -e .`(无 extras)仍能跑 `init/write/search/maintain/doctor`,不报缺包
9. 触发 `import` extras 缺失时给出友好错误而非 trace
10. 现有 4 个 hooks 的行为在 embedder/rerank 禁用下完全不变

---

## 12. 参考

- TencentDB-Agent-Memory:L0-L3 分层、BM25+vector RRF、Mermaid 符号化记忆(下一轮可借鉴抽象金字塔)
- mem0:vector+graph 混合 API
- letta (MemGPT):agent-state 记忆,虚拟上下文管理
- Zep:时间知识图谱
- MCP Specification:https://spec.modelcontextprotocol.io/

---

## 13. 风险与缓解

| 风险 | 缓解 |
|---|---|
| ONNX 模型首次下载慢 | 文档化 onnx_path 手动指向,init/doctor 报告下载状态 |
| 1 万条以上 corpus 的 Python cosine 慢 | 文档化 numpy extras + 测试;>10 万再考虑 sqlite-vec |
| MCP SDK 版本破坏性升级 | 在 `pyproject.toml` 锁定主版本范围 `mcp>=0.4,<1` |
| 链接图过深扩展爆炸 | 默认 `max_hops=1`;`graph CLI` 默认 `depth=2` 并有上限 |
| 已有 link 用了非预定义 rel | 静默接受,按 fallback 0.5 衰减;`link --allow-custom` 显式声明 |

---

(End of spec)
