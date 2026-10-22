import { useCallback, useEffect, useRef, useState } from "react";
import { onTaskProgress } from "./api";
import type { TaskKind } from "./types";

export interface TaskProgress {
  task: TaskKind;
  done: number;
  /** 0 mientras no se sabe cuánto falta. */
  total: number;
}

/**
 * Progreso de una tarea larga (copia, restauración, análisis, importación)
 * mientras la promesa del comando sigue pendiente.
 */
export function useTask(serverId: string | null) {
  const [progress, setProgress] = useState<TaskProgress | null>(null);
  // Los eventos que lleguen después de terminar no deben revivir la barra.
  const active = useRef<TaskKind | null>(null);

  useEffect(() => {
    const off = onTaskProgress((p) => {
      if (active.current === p.task && p.serverId === serverId) {
        setProgress({ task: p.task, done: p.done, total: p.total });
      }
    });
    return () => {
      off.then((f) => f());
    };
  }, [serverId]);

  const run = useCallback(async <T>(task: TaskKind, action: () => Promise<T>): Promise<T> => {
    active.current = task;
    setProgress({ task, done: 0, total: 0 });
    try {
      return await action();
    } finally {
      active.current = null;
      setProgress(null);
    }
  }, []);

  return { progress, run };
}
