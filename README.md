# Godot Hub

Ứng dụng Windows quản lý và khởi chạy nhiều bản Godot Engine song song. Godot Hub dùng Tauri 2, React, TypeScript và Rust, lấy danh mục trực tiếp từ [Godot Builds trên GitHub](https://github.com/godotengine/godot-builds/releases).

## Cài đặt và sử dụng

1. Chạy `Godot Hub_0.1.0_x64-setup.exe` trên Windows 10/11 x64. Bộ cài NSIS đăng ký theo tài khoản hiện tại, tạo mục Start Menu và chọn sẵn shortcut Desktop. Có thể gỡ từ **Settings → Apps → Installed apps**. Bản build local chưa được ký Authenticode.
2. Mở **Installs → Available releases**. Chọn kênh Stable, Beta, RC hoặc Dev, rồi chọn Standard hoặc .NET. Hub mặc định chọn kiến trúc tương ứng với máy.
3. Bấm **Install**. Xem tiến trình tại **Downloads**. Khi hoàn tất, bấm **Open** tại **Installs → Installed** để mở trình quản lý project của engine đó.
4. Dùng **Import engine** để thêm một file `.exe` đã giải nén. Hub đọc phiên bản và edition từ kết quả `--help` của file được chọn. Menu của mỗi engine có đặt mặc định, mở thư mục, tạo shortcut Desktop, cài Export Templates và gỡ engine. Gỡ một engine đã import chỉ xóa đăng ký trong Hub.
5. Dùng **Settings** để chọn thư mục engine, thư mục project mặc định, giao diện và kiểm tra Godot mới khi mở Hub. Với bản .NET, cài [.NET SDK tương thích](https://docs.godotengine.org/en/stable/tutorials/scripting/c_sharp/c_sharp_basics.html) riêng để phát triển project C#.

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

Đường dẫn mặc định `src-tauri/target/release/bundle/nsis/Godot Hub_0.1.0_x64-setup.exe`.

```powershell
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
```

## Kiến trúc và cây thư mục

```text
GodotHub/
├── src/
│   ├── api.ts, useHub.ts         # IPC, state ứng dụng và tiến trình
│   ├── App.tsx, styles.css       # Shell desktop, theme và bố cục
│   └── views/                   # Projects, Installs, Downloads, Settings
├── public/                      # Icon giao diện
└── src-tauri/
    ├── tauri.conf.json           # Cửa sổ, CSP, bộ cài NSIS per-user
    ├── capabilities/, icons/    # Quyền WebView và icon Windows
    └── src/
        ├── main.rs               # Lệnh IPC, điều phối luồng cài
        ├── catalog.rs            # GitHub Releases, phân loại, cache và ETag
        ├── download.rs, jobs.rs  # Download, checksum và phát tiến trình
        ├── archive.rs, engine.rs # Giải nén, registry engine, export templates
        └── storage.rs, project.rs, windows_host.rs
```

- `storage.rs` lưu cấu hình, registry engine và ID engine mặc định trong `%APPDATA%\com.godothub.app\state.json`. Cache danh mục và engine mặc định nằm trong `%LOCALAPPDATA%\com.godothub.app`.
- Mỗi job tải vào thư mục tạm cùng ổ đĩa đích, đối chiếu kích thước và checksum chính thức nếu có, rồi giải nén. Hub kiểm tra đường dẫn ZIP, chặn liên kết và bỏ dữ liệu tạm khi job kết thúc. Chỉ đăng ký engine sau khi tìm được executable Windows đúng kiến trúc.
- Templates dùng đường dẫn Godot đọc được, ví dụ `%APPDATA%\Godot\export_templates\4.7.2.stable`. Engine self-contained dùng `editor_data` cạnh executable. Godot 2 do Hub cài dùng self-contained vì engine dòng này dùng một thư mục templates chung.
- Cài phiên bản mới luôn tạo thư mục engine riêng. Đổi thư mục engine trong Settings áp dụng cho các lần cài tiếp theo. Default và lần cài từng phiên bản được lưu bằng ID riêng, sẵn sàng để Project Manager gắn từng project với một engine.
- `project.rs` là điểm gọi tiến trình cho engine và cho project sau này. Plugin Manager và RPG Toolkit có thể thêm module riêng, dùng lại lớp IPC và lưu trữ. Tab **Projects** hiện hiển thị trạng thái chờ Phase 4.

## Phạm vi lộ trình

Phase 1 gồm desktop shell, cấu hình, theme, icon và bộ cài. Phase 2 gồm danh mục thật, cài và quản lý engine, import, export templates, shortcut và tiến trình. Giao diện đã có release notes và báo phiên bản stable mới cho engine mặc định. Update Manager đầy đủ, quản lý project trong Hub, Plugin Manager và cập nhật chính Godot Hub nằm ở các phase kế tiếp.
