# Godot Hub

Ứng dụng Windows quản lý nhiều bản Godot Engine song song và mở từng project bằng engine đã chọn. Godot Hub dùng Tauri 2, React, TypeScript và Rust, lấy danh mục trực tiếp từ [Godot Builds trên GitHub](https://github.com/godotengine/godot-builds/releases).

## Cài đặt và sử dụng

1. Chạy `Godot Hub_0.4.0_x64-setup.exe` trên Windows 10/11 x64. Bộ cài NSIS đăng ký theo tài khoản hiện tại, tạo mục Start Menu và chọn sẵn shortcut Desktop. Có thể gỡ từ **Settings → Apps → Installed apps**. Chữ ký Tauri dùng cho cập nhật tự động. Bản build local chưa được ký Authenticode.
2. Mở **Installs → Available releases**. Chọn kênh Stable, Beta, RC hoặc Dev, rồi chọn Standard hoặc .NET. Hub mặc định chọn kiến trúc tương ứng với máy.
3. Bấm **Install**. Xem tiến trình tại **Downloads**. Khi hoàn tất, bấm **Open** tại **Installs → Installed** để mở trình quản lý project của engine đó.
4. Dùng **Import engine** để thêm một file `.exe` đã giải nén. Hub đọc phiên bản và edition từ kết quả `--help` của file được chọn. Menu của mỗi engine có đặt mặc định, mở thư mục, tạo shortcut Desktop, cài Export Templates và gỡ engine. Gỡ một engine đã import chỉ xóa đăng ký trong Hub.
5. Vào **Installs → Updates** để kiểm tra hoặc xem các bản cập nhật, mở **Changelog** và chọn **Install alongside**. Tùy chọn **Settings → Check for engine updates at startup** điều khiển lần kiểm tra khi mở Hub. Với bản .NET, cài [.NET SDK tương thích](https://docs.godotengine.org/en/stable/tutorials/scripting/c_sharp/c_sharp_basics.html) riêng để phát triển project C#.

Update Manager hiển thị bản mới phù hợp với edition và kiến trúc đang cài. Engine Stable nhận bản Stable mới nhất cùng major. Engine thử nghiệm nhận bản mới cùng major.minor, bao gồm Stable chính thức của nhánh đó. Hub xác thực lại danh mục GitHub khi kiểm tra và báo rõ khi dùng cache. Cài cập nhật tạo thư mục engine riêng, giữ engine mặc định, các bản đã cài và project hiện có.

## Quản lý project

1. Vào **Projects → New project**, đặt tên và chọn engine. Hub tạo project trống trong thư mục con của vị trí đã chọn. Vị trí mặc định lấy từ Settings.
2. Dùng **Import project**, chọn `project.godot` hoặc `engine.cfg` của Godot 2. Nếu import lại cùng đường dẫn, Hub dùng lại mục đã đăng ký.
3. Chọn engine cho mỗi project rồi bấm **Open**. Danh sách xếp theo lần mở thành công gần nhất. Nút Explorer mở thư mục chứa project.

Project lưu ID engine đã chọn. Khi gỡ engine hoặc đổi engine mặc định, project vẫn trỏ đến ID cũ. Hub yêu cầu chọn lại nếu engine đó mất, và kiểm tra major cũng như yêu cầu .NET trước khi mở.

**Projects** tự tìm các project có trong danh sách Godot 3/4 và trong thư mục project mặc định. Để thêm cả nhóm project ở vị trí khác, dùng **Scan folder**. Kết quả tổng hợp vào danh sách Hub và có thể lọc theo tên hoặc đường dẫn. Hub bỏ qua thư mục addon, cache và thư mục engine khi quét.

## Plugin Manager

1. Vào **Plugins** và chọn một project đã thêm vào Hub.
2. Chọn **Import ZIP** hoặc tìm addon trong **Asset Library** chính thức. Hub nhận các plugin Godot có `addons/<tên>/plugin.cfg` và cài vào project. Thư mục addon hiện có được giữ nguyên nếu trùng tên.
3. Bật addon trong **Godot Editor → Project Settings → Plugins**. Khi gỡ addon do Hub cài, Hub chuyển cây thư mục addon sang `.godot-hub-plugin-backups` trong project để có thể khôi phục.

## Cập nhật Godot Hub

Từ 0.4.0, **Automatically update Godot Hub** trong Settings sẽ kiểm tra GitHub lúc mở Hub, tải bộ cài đã ký, rồi cài và khởi động lại khi các tác vụ trong Hub rảnh. Có thể kiểm tra hoặc cài thủ công trong **Settings → Godot Hub updates**. Các bản 0.1–0.3 cần chạy bộ cài mới một lần để nhận chức năng này.

Workflow `.github/workflows/release-windows.yml` build và ký NSIS khi push tag `vX.Y.Z` khớp phiên bản trong source. Để bật phát hành, cấu hình GitHub Actions secrets `TAURI_SIGNING_PRIVATE_KEY` và `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` với cặp khóa khớp `plugins.updater.pubkey`. Workflow tải EXE, chữ ký `.sig` và `latest.json` trước khi công bố release. Khóa bí mật lưu ngoài repo. Feed updater đọc `latest.json` của release công khai mới nhất.

## Build trên Windows

Chuẩn bị Node.js 22.12+ (hoặc 24), Rust stable 1.90+, Microsoft C++ Build Tools với workload Desktop development with C++ và Windows SDK. Windows cần WebView2 Runtime. Bộ cài sẽ tải bootstrapper WebView2 nếu cần. [Yêu cầu Tauri](https://v2.tauri.app/start/prerequisites/)

```powershell
npm ci
npm run tauri dev
```

Đóng phiên dev, sau đó build bộ cài.

```powershell
npm run tauri build -- --ci
```

Đường dẫn mặc định `src-tauri/target/release/bundle/nsis/Godot Hub_0.4.0_x64-setup.exe`.

```powershell
npm run build
npm test
cargo test --manifest-path src-tauri/Cargo.toml
```

## Kiến trúc và cây thư mục

```text
GodotHub/
├── src/
│   ├── api.ts, useHub.ts         # IPC, state ứng dụng và tiến trình
│   ├── updates.ts               # Chọn cập nhật theo phiên bản, edition và kiến trúc
│   ├── App.tsx, styles.css       # Shell desktop, theme và bố cục
│   └── views/                   # Projects, Installs, Plugins, Downloads, Settings
├── public/                      # Icon giao diện
└── src-tauri/
    ├── tauri.conf.json           # Cửa sổ, CSP, bộ cài NSIS per-user
    ├── capabilities/, icons/    # Quyền WebView và icon Windows
    └── src/
        ├── main.rs               # Lệnh IPC, điều phối luồng cài
        ├── catalog.rs            # GitHub Releases, phân loại, cache và ETag
        ├── download.rs, jobs.rs  # Download, checksum và phát tiến trình
        ├── archive.rs, engine.rs # Giải nén, registry engine, export templates
        ├── storage.rs, project.rs, project/discovery.rs, windows_host.rs
        └── plugins.rs, self_update.rs
```

- `%APPDATA%\com.godothub.app\state.json` lưu cấu hình, registry engine, engine mặc định và danh sách project. Cache danh mục và thư mục cài engine mặc định nằm trong `%LOCALAPPDATA%\com.godothub.app`.
- Mỗi job tải vào thư mục tạm cùng ổ đĩa đích, đối chiếu kích thước và checksum chính thức nếu có, rồi giải nén. Hub kiểm tra đường dẫn ZIP, chặn liên kết và bỏ dữ liệu tạm khi job kết thúc. Chỉ đăng ký engine sau khi tìm được executable Windows đúng kiến trúc.
- Templates dùng đường dẫn Godot đọc được, ví dụ `%APPDATA%\Godot\export_templates\4.7.2.stable`. Engine self-contained dùng `editor_data` cạnh executable. Godot 2 do Hub cài dùng self-contained vì engine dòng này dùng một thư mục templates chung.
- Cài phiên bản mới luôn tạo thư mục engine riêng. Đổi thư mục engine trong Settings áp dụng cho các lần cài tiếp theo.
- `project.rs` tạo, đọc và mở project bằng engine đã gán. Plugin Manager và RPG Toolkit có thể thêm module riêng và dùng lại lớp IPC và lưu trữ.

## Phạm vi lộ trình

Phase 1 gồm ứng dụng desktop và bộ cài. Phase 2 bổ sung Engine Manager. Phase 3 bổ sung Update Manager cho Godot Engine. Phase 4 bổ sung Project Manager. Phase 5 thêm Plugin Manager, tự cập nhật Godot Hub và tìm project hiện có.
