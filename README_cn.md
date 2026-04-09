# infinishield

[English](README.md) | [简体中文](README_cn.md)

[![Build](https://github.com/infinilabs/infinishield/actions/workflows/build.yml/badge.svg)](https://github.com/infinilabs/infinishield/actions/workflows/build.yml)

一个用于将隐形水印嵌入到图像、SVG 和视频中的命令行工具。

## 功能介绍

infinishield 可以将简短的文本信息（例如版权声明）隐藏在文件内部。该水印是不可见的、受密码保护的，并且能够抵抗如裁剪和压缩等常见的修改操作。

## 支持的格式

| 格式 | 状态 | 最大消息长度 |
|--------|--------|:-----------:|
| JPEG, PNG, WebP, BMP, TIFF, GIF | 支持 | 7 字节（抗裁剪）或 ~40 字节（较长，无抗裁剪保护） |
| SVG | 支持 | 2-7 字节（取决于路径复杂度） |
| MP4, WebM, MOV, AVI, MKV | 支持（可选编译） | 7 字节 |

## 编译安装

需要 [Rust](https://rustup.rs/) 1.70+。

```bash
make release          # 仅支持图像 + SVG
make release-video    # 支持图像 + SVG + 视频（详见下文）
```

### 视频支持（可选）

视频功能需要额外的系统库，并且编译时间较长（会从源码编译 FFmpeg）：

```bash
# Linux (Debian/Ubuntu)
sudo apt-get install nasm pkg-config gcc make libx264-dev

# macOS
brew install nasm pkg-config x264

# 然后编译
make release-video
```

## 使用方法

### 嵌入水印

```bash
infinishield embed -i photo.jpg -o watermarked.png                      # 图像
infinishield embed -i logo.svg -o logo_wm.svg -m "Hi"                   # SVG
infinishield embed -i clip.mp4 -o watermarked.mp4                       # 视频
infinishield embed -i photo.jpg -m "MyMark" -p "secret" -o out.png      # 自定义消息 + 密码
infinishield embed -i photo.jpg -o out.png --dry-run                    # 仅预览
```

| 选项 | 是否必须 | 默认值 | 描述 |
|--------|----------|---------|-------------|
| `-i` | 是 | — | 输入文件 |
| `-o` | 是 | — | 输出文件 |
| `-m` | 否 | `"Infini"` | 要嵌入的消息 |
| `-p` | 否 | `"d1ng0"` | 密码 |
| `--intensity` | 否 | 自动 | 强度 1-10（仅限图像，见下文） |
| `--dry-run` | 否 | — | 仅预览，不写入文件 |

**强度 (Intensity):** 如果省略此参数，infinishield 会根据图像大小使用对数曲线自动选择最佳强度——较小的图像使用较浅的水印以保持隐蔽性，较大的图像使用较强的水印。当手动设置时（`--intensity 1..10`），您的设置会作为自动曲线的相对乘数系数（1 = 50%, 5 = ~自动, 10 = 200%），因此水印的绝对强度仍然会自适应图像大小。

### 验证/提取水印

```bash
infinishield verify -i watermarked.png
infinishield verify -i watermarked.png -p "secret"
```

### 帮助

```bash
infinishield              # 完整帮助
infinishield --version    # 版本信息
```

## 资产准备最佳实践

**图像 (JPEG/PNG):**
- 使用 PNG 输出（`-o out.png`）以获得最佳的水印保存效果。JPEG 输出是有损压缩，会使水印退化。
- 最多 7 字节的消息（例如 `"Infini"`）可以抗裁剪。对于更长的消息，将失去抗裁剪保护。
- 图像尺寸应至少为 512×512 像素。较大的图像通常会产生视觉影响更小的完美结果。
- 旋转和缩放操作会破坏水印——目前算法仅能抵抗裁剪和压缩攻击。

**SVG:**
- 在包含复杂路径（插画、带曲线的图标等）的 SVG 上效果最好。简单的几何形状（普通的矩形、圆形）包含的坐标太少，无法嵌入足够的信息。
- 建议先使用 `--dry-run` 检查您的 SVG 是否有足够符合条件的路径。
- 不要使用会重新格式化或对坐标值进行四舍五入的编辑器打开已加水印的 SVG——这会破坏水印的量化数据。以只读方式查看或程序化渲染是安全的。

**视频:**
- 请保持消息简短（≤ 7 字节）。水印以每秒 1 个关键帧的频率进行嵌入。
- 输出视频默认使用 H.264 (CRF 18) 重新编码。不保留原始编解码器格式。
- 在某些包含复杂场景的视频上，重新编码期间可能会有 1-2 bit 发生翻转。虽然检测仍然可靠，但不能始终保证能完全准确地恢复原始消息字节。
- 采用流式架构——内存中同时只保留 1-2 帧。可从容处理任意长度的高清视频，不会导致内存溢出。
- 编译的二进制文件包含静态链接的 FFmpeg——不需要在运行环境中额外安装依赖。

## Web UI

提供了一个基于浏览器的测试界面，用于嵌入和验证水印。需要 [Go](https://go.dev/) 1.21+ 环境。

```bash
make webapp     # 构建并运行在 http://localhost:1983
```

功能特点：
- 上传要加水印的图像、SVG 或视频
- 配置消息、密码和强度，提供实时的 dry-run 预览
- 左右分屏对比原始资产和已加水印的资产
- 独立的验证选项卡，用于从未知文件中提取水印
- 将运行事件日志记录到 `webapp/logs/` 目录

## 运行测试

```bash
make test       # 运行图像 + SVG 测试
make sanity     # 完整检查: format + lint + build + 所有测试（包含视频测试）
make clean      # 移除构建产物
```

## 常见问题与排错

### 嵌入后出现可见噪点或伪影

水印修改的是绿色通道中的像素值。在某些图像上——尤其是尺寸极小的图像或具有大面积平坦/均匀颜色的区域（天空、纯色背景、简单插图）——这可能会产生微弱的可见噪点（类似胶片颗粒）。

**请按顺序尝试以下步骤：**

1. **使用更低的强度。** 默认的自动模式旨在取得平衡，但您可以手动降低它：
   ```bash
   infinishield embed -i photo.jpg -o out.png --intensity 3
   infinishield embed -i photo.jpg -o out.png --intensity 1   # 最低强度
   ```
   较低的强度 = 噪点更少，但水印也更弱。可以在生成文件前使用 `--dry-run` 预览强度参数。

2. **使用更大的源图像。** 水印被分散在较大图像的更多像素上，使得每个像素的修改更不易察觉。如果可能，请对全分辨率原图加水印，而不是对调整大小后的缩略图加水印。不建议对低于 512×512 像素的图像进行处理。

3. **检查输出格式。** 务必使用 PNG 输出（`-o out.png`）。JPEG 的有损压缩不仅会降低水印鲁棒性，还会放大水印产生的视觉伪影。输入文件可以是任何支持的格式。

4. **先尝试 `--dry-run`。** 预览嵌入参数而不实际写入文件：
   ```bash
   infinishield embed -i photo.jpg -o out.png --dry-run
   ```
   这会显示系统选择的模式、内部强度、关键点数量和容量上限——有助于理解工具的行为。

5. **了解图像特征。** 空间域水印天然在纹理丰富、细节繁多的区域（风景、照片）中更难被察觉，而在平坦、均匀的区域（Logo、线稿、纯色背景）中更容易显现。这是该技术固有的特性。

### 裁剪后未检测到水印

- 只有不超过 **7 字节** 的消息才支持抗裁剪保护（即使用特征点模式）。更长的消息会自动切换使用全局 DWT 模式，该模式没有抗裁剪能力。
- 裁剪操作必须保留足够多的关键点区域。非常激进的裁剪（例如切掉原图 > 75% 的面积）可能会移除过多的特征锚点，导致无法恢复。
- 缩放（Resize）或旋转（Rotate）图像**一定**会破坏水印——该算法目前仅能抵抗裁剪（Crop）和压缩（Compression）攻击。

### 提取出的消息不正确

- 确保您在嵌入和验证时使用了完全相同的密码（`-p` 标志）。
- 在极大的图像上使用非常低的强度（`--intensity 1`）可能会导致偶发的比特错误。请使用更高的强度或默认的自动模式以确保可靠提取。
- JPEG 输出会使水印严重退化。测试时请始终基于导出的 PNG 格式进行验证，而不是被重新编码过的 JPEG。

### 视频水印问题

- 视频由于强行使用 H.264 (CRF 18) 重新编码。在极端复杂的运动场景中，可能有个别比特发生翻转——“是否包含水印”的检测判定仍然是可靠的，但由于校验错乱，并不总是能完美恢复最初的消息字节。
- 如果您正在对被裁剪（截取时间段）的视频进行验证，请确保保留的片段中至少包含几个带有水印的关键帧。系统是每隔一秒钟选取一帧进行嵌入的。

## 更多文档

- [技术细节 (Technical Details)](docs/tech_details.md) — 了解底层架构、算法选型、哪些方案有效以及为何放弃了其他方案。
