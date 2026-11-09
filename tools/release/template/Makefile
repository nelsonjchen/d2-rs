.POSIX:
.SILENT:

.PHONY: all
all:
	echo "usage: make install | make uninstall  (PREFIX=dir, DRY_RUN=1)" >&2
	exit 1

.PHONY: install
install:
	./scripts/install.sh

.PHONY: uninstall
uninstall:
	./scripts/uninstall.sh
