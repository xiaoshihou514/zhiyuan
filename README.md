<div align="center">

# 致远

<img src="./template/icon.svg" alt="logo" width="30%" />

致远是一个开源的“深度研究”框架。

</div>

<img width="1919" height="1037" alt="Image" src="https://github.com/user-attachments/assets/4b18f1bd-6323-4323-a1e1-f90803134eda" />

```shell
zhiyuan "某话题"
zhiyuan --long "某复杂话题"
```

输出示例：
- [东亚共同体的一体化前景_共同叙事_利益驱动力与现实挑战.pdf](https://github.com/user-attachments/files/30166087/_._.pdf)：“平等的东亚共同体的共同叙事、建构和联合的利益驱动力。经济与意识形态可能的现实东亚一体化方案（类似欧共体）和东亚一体化挑战”，普通模式，deepseek-v4-flash
- [东亚经济 2025：趋势、挑战与转型路径.pdf](https://github.com/user-attachments/files/30166090/2025.pdf)：“东亚经济2025”，深度模式，deepseek-v4-flash
- [大模型辅助科研在组合数学方向前沿研究的应用.pdf](https://github.com/user-attachments/files/30166092/default.pdf)：“大模型辅助科研在组合数学方向前沿研究的应用”，普通模式，deepseek-v4-flash

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
