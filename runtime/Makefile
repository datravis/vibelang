CC ?= gcc
CFLAGS = -O2 -Wall -Wextra -std=c11 -fPIC -D_GNU_SOURCE
LDFLAGS = -lpthread

# Output: static library for linking with compiled VibeLang programs
TARGET = libvibe_runtime.a
SHARED = libvibe_runtime.so

SOURCES = vibe_runtime.c
OBJECTS = $(SOURCES:.c=.o)

.PHONY: all clean

all: $(TARGET) $(SHARED)

$(TARGET): $(OBJECTS)
	ar rcs $@ $^

$(SHARED): $(OBJECTS)
	$(CC) -shared -o $@ $^ $(LDFLAGS)

%.o: %.c vibe_runtime.h
	$(CC) $(CFLAGS) -c $< -o $@

clean:
	rm -f $(OBJECTS) $(TARGET) $(SHARED)
