# nmea-monitor

`cat /dev/ttyUSB0 | nmea-monitor` みたいな感じで呼び出して、経緯度や Fix しているかなどを TUI に表示更新し続けたいやつ

## 構想

- 標準入力から NMEA0183 を受け取って、 GNSS 周りの情報を保持する
- GNSS の状態を TUI に表示する
- `--web` を与えた場合は位置情報を leaflet だとかを使ってブラウザ上にプロットする
