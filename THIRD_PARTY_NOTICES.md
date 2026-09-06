# Third-party notices

## Bootstrap Icons

The settings, panel-tab, volume, copy, play and remove glyphs are from Bootstrap Icons 1.13.1.

Copyright 2019-2024 The Bootstrap Authors. Licensed under the MIT License.

Project: <https://icons.getbootstrap.com/>

## Keyring ecosystem

Qiliu uses `keyring-core` and the native Apple, Windows and Android credential-store providers to keep Bilibili login material outside the web frontend.

Copyright their respective contributors. Licensed under MIT OR Apache-2.0.

Projects:

- <https://github.com/open-source-cooperative/keyring-core>
- <https://github.com/open-source-cooperative/apple-native-keyring-store>
- <https://github.com/open-source-cooperative/windows-native-keyring-store>
- <https://github.com/open-source-cooperative/android-native-keyring-store>


## DouyinLiveRecorder

Douyu request signing and Douyin stream parsing follow the public implementation by Hmily. The Douyin a_bogus implementation is ported from `src/ab_sign.py`. No account cookies from the reference project are included.

Project: <https://github.com/ihmily/DouyinLiveRecorder>

MIT License

Copyright (c) 2025 Hmily

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.


## Boa and RustCrypto SM3

Boa provides an embedded JavaScript interpreter for Douyu signing, without host filesystem or network bindings. Licensed under MIT OR Unlicense.

Project: <https://github.com/boa-dev/boa>

RustCrypto SM3 provides the hash used by Douyin request signing. Licensed under MIT OR Apache-2.0.

Project: <https://github.com/RustCrypto/hashes>
