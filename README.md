# 致远

## 配置
```toml
[search]
max_results = 10                      # 每个搜索引擎最大结果数
searxng_url = "http://localhost:8888" # SearXNG 实例地址，默认使用本地docker
blocked_domains = []

[llm]
api_key = ""
base_url = "https://api.deepseek.com/v1"
main_model = "deepseek-v4-flash"

[research]
concurrency = 4

[pdf]
font_paths = []
```

如果你在Linux上，可以考虑常见的Noto Sans系列字体。
```shell
fc-list | rg 'Noto Sans' | cut -d':' -f1 | sort | uniq | rg 'Regular'
```
