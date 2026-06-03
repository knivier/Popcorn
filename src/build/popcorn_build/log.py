from __future__ import annotations

from dataclasses import dataclass
from threading import Lock
from typing import Literal


Level = Literal["INFO", "OK", "WARN", "ERROR", "CMD"]


@dataclass(frozen=True)
class LogEvent:
    idx: int
    level: Level
    message: str


class LogBuffer:
    def __init__(self, max_events: int = 5000):
        self._lock = Lock()
        self._events: list[LogEvent] = []
        self._next_idx = 0
        self._max = max_events

    def add(self, level: Level, message: str) -> LogEvent:
        with self._lock:
            ev = LogEvent(idx=self._next_idx, level=level, message=message)
            self._next_idx += 1
            self._events.append(ev)
            if len(self._events) > self._max:
                self._events = self._events[-self._max :]
            return ev

    def snapshot(self, since_idx: int) -> list[LogEvent]:
        with self._lock:
            return [e for e in self._events if e.idx > since_idx]

    def clear(self) -> None:
        with self._lock:
            self._events = []
            self._next_idx = 0

