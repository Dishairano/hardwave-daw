Helpers that host one plug-in in a process of their own.

`hardwave-plugin-bridge-x86.exe` is the 32-bit build. A 64-bit process
cannot load a 32-bit plug-in, so plug-ins that were never rebuilt run
in here and speak to the app over a pipe. CI builds it on Windows
before the installer is bundled; this folder is otherwise empty, which
is why the app treats a missing helper as "no helper" rather than an
error.
