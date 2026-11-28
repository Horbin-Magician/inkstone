//! Per-workspace FIFO for independent background reads. Only the foreground
//! completion may publish a result, after cancellation and vault checks.
use super::*;
use gpui_component::button::{Button, ButtonVariants};
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};

const CONCURRENCY: usize = 2;
const HISTORY: usize = 100;
type Start = Box<dyn FnOnce(usize, &mut Context<Workspace>)>;
type Retry = Rc<dyn Fn(&mut Workspace, &mut Window, &mut Context<Workspace>)>;

pub(super) struct Progress {
    cancelled: Arc<AtomicBool>,
    stages: async_channel::Sender<&'static str>,
}
impl Progress {
    pub fn stage(&self, label: &'static str) -> anyhow::Result<()> {
        anyhow::ensure!(!self.cancelled.load(Ordering::Acquire), "已取消");
        self.stages
            .try_send(label)
            .map_err(|_| anyhow::anyhow!("工作区已关闭"))
    }
}

enum State {
    Queued,
    Running(&'static str),
    Cancelling,
    Cancelled,
    Finished,
    Failed(String),
}
struct Job {
    key: String,
    label: String,
    state: State,
    cancelled: Arc<AtomicBool>,
    start: Option<Start>,
    retry: Retry,
}
#[derive(Default)]
pub(super) struct Queue {
    next: usize,
    running: usize,
    waiting: VecDeque<usize>,
    active_keys: HashSet<String>,
    jobs: BTreeMap<usize, Job>,
    history: VecDeque<usize>,
}
impl Queue {
    pub fn active_count(&self) -> usize {
        self.active_keys.len()
    }

    pub fn reset(&mut self) {
        for job in self.jobs.values() {
            job.cancelled.store(true, Ordering::Release);
        }
        self.jobs.clear();
        self.waiting.clear();
        self.active_keys.clear();
        self.history.clear();
        // Old workers still occupy slots until their callbacks return.
    }

    fn remember(&mut self, id: usize) {
        self.history.push_back(id);
        if self.history.len() > HISTORY
            && let Some(old) = self.history.pop_front()
        {
            self.jobs.remove(&old);
        }
    }

    fn cancel(&mut self, id: usize) {
        let Some(job) = self.jobs.get_mut(&id) else {
            return;
        };
        job.cancelled.store(true, Ordering::Release);
        match job.state {
            State::Queued => {
                job.start.take();
                job.state = State::Cancelled;
                self.active_keys.remove(&job.key);
                self.remember(id);
            }
            State::Running(_) => job.state = State::Cancelling,
            _ => {}
        }
    }
}

impl Workspace {
    pub(super) fn enqueue_task<T: Send + 'static>(
        &mut self,
        key: String,
        label: String,
        work: impl FnOnce(Progress) -> anyhow::Result<T> + Send + 'static,
        apply: impl FnOnce(
            &mut Workspace,
            anyhow::Result<T>,
            &mut Context<Workspace>,
        ) -> anyhow::Result<()>
        + 'static,
        retry: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
        cx: &mut Context<Self>,
    ) {
        if !self.background_tasks.active_keys.insert(key.clone()) {
            return;
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        let start: Start = Box::new(move |id, cx| {
            let (stages, updates) = async_channel::unbounded();
            let task = cx.background_executor().spawn(async move {
                anyhow::ensure!(!flag.load(Ordering::Acquire), "已取消");
                work(Progress {
                    cancelled: flag,
                    stages,
                })
            });
            cx.spawn(async move |this, cx| {
                while let Ok(label) = updates.recv().await {
                    let _ = this.update(cx, |this, cx| {
                        if let Some(job) = this.background_tasks.jobs.get_mut(&id)
                            && matches!(job.state, State::Running(_))
                        {
                            job.state = State::Running(label);
                            cx.notify();
                        }
                    });
                }
                let result = task.await;
                let _ = this.update(cx, |this, cx| {
                    this.background_tasks.running -= 1;
                    if let Some(job) = this.background_tasks.jobs.get(&id) {
                        let cancelled = job.cancelled.load(Ordering::Acquire);
                        let key = job.key.clone();
                        let state = if cancelled {
                            State::Cancelled
                        } else {
                            match apply(this, result, cx) {
                                Ok(()) => State::Finished,
                                Err(error) => State::Failed(format!("失败：{error}")),
                            }
                        };
                        this.background_tasks.jobs.get_mut(&id).unwrap().state = state;
                        this.background_tasks.active_keys.remove(&key);
                        this.background_tasks.remember(id);
                    }
                    this.pump_tasks(cx);
                    cx.notify();
                });
            })
            .detach();
        });
        let queue = &mut self.background_tasks;
        let id = queue.next;
        queue.next += 1;
        queue.jobs.insert(
            id,
            Job {
                key,
                label,
                state: State::Queued,
                cancelled,
                start: Some(start),
                retry: Rc::new(retry),
            },
        );
        queue.waiting.push_back(id);
        self.pump_tasks(cx);
        cx.notify();
    }

    fn pump_tasks(&mut self, cx: &mut Context<Self>) {
        let queue = &mut self.background_tasks;
        while queue.running < CONCURRENCY {
            let Some(id) = queue.waiting.pop_front() else {
                break;
            };
            let Some(job) = queue.jobs.get_mut(&id) else {
                continue;
            };
            if let Some(start) = job.start.take() {
                job.state = State::Running("运行中");
                queue.running += 1;
                start(id, cx);
            }
        }
    }

    pub(super) fn show_background_tasks(&mut self, cx: &mut Context<Self>) {
        self.ui.focus_mode = false;
        self.ui.prefs.right_open = true;
        self.ui.right_mode = 6;
        self.persist_workspace(cx);
        cx.notify();
    }

    pub(super) fn background_task_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let queue = &self.background_tasks;
        let mut ids = Vec::with_capacity(queue.jobs.len());
        ids.extend(
            queue
                .jobs
                .iter()
                .filter(|(_, j)| matches!(j.state, State::Running(_) | State::Cancelling))
                .map(|(id, _)| *id),
        );
        ids.extend(queue.waiting.iter().copied().filter(|id| {
            queue
                .jobs
                .get(id)
                .is_some_and(|j| matches!(j.state, State::Queued))
        }));
        ids.extend(queue.history.iter().rev().copied());
        div()
            .flex()
            .flex_col()
            .p_2()
            .min_h_0()
            .flex_1()
            .child(
                div()
                    .p_2()
                    .font_weight(FontWeight::SEMIBOLD)
                    .flex()
                    .items_center()
                    .justify_between()
                    .child("后台任务")
                    .when(!queue.history.is_empty(), |s| {
                        s.child(
                            Button::new("clear-task-history")
                                .ghost()
                                .label("清除记录")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let queue = &mut this.background_tasks;
                                    for id in queue.history.drain(..) {
                                        queue.jobs.remove(&id);
                                    }
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .when(ids.is_empty(), |s| {
                s.child(div().p_2().child("暂无后台任务"))
            })
            .child(
                uniform_list(
                    "background-tasks",
                    ids.len(),
                    cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                        range
                            .filter_map(|index| {
                                let id = ids[index];
                                let job = this.background_tasks.jobs.get(&id)?;
                                let status = match &job.state {
                                    State::Queued => "排队中",
                                    State::Running(label) => *label,
                                    State::Cancelling => "正在取消",
                                    State::Cancelled => "已取消",
                                    State::Finished => "已完成",
                                    State::Failed(message) => message.as_str(),
                                };
                                let retry = job.retry.clone();
                                Some(
                                    div()
                                        .h(px(68.))
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .px_2()
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .flex()
                                                .flex_col()
                                                .child(div().truncate().child(job.label.clone()))
                                                .child(
                                                    div()
                                                        .id(("task-status", id))
                                                        .overflow_y_scroll()
                                                        .max_h(px(40.))
                                                        .text_size(px(12.))
                                                        .child(status.to_owned()),
                                                ),
                                        )
                                        .when(
                                            matches!(job.state, State::Running(_) | State::Queued),
                                            |s| {
                                                s.child(
                                                    Button::new(("cancel-task", id))
                                                        .ghost()
                                                        .label("取消")
                                                        .on_click(cx.listener(
                                                            move |this, _, _, cx| {
                                                                this.background_tasks.cancel(id);
                                                                cx.notify();
                                                            },
                                                        )),
                                                )
                                            },
                                        )
                                        .when(
                                            matches!(
                                                job.state,
                                                State::Failed(_) | State::Cancelled
                                            ),
                                            |s| {
                                                s.child(
                                                    Button::new(("retry-task", id))
                                                        .ghost()
                                                        .label("重试")
                                                        .on_click(cx.listener(
                                                            move |this, _, w, cx| {
                                                                retry(this, w, cx)
                                                            },
                                                        )),
                                                )
                                            },
                                        )
                                        .into_any_element(),
                                )
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .flex_1(),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[gpui::test]
    fn cancelled_jobs_never_publish_and_release_their_keys(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let window = cx.add_window(Workspace::new);
        window
            .update(cx, |w, _, cx| {
                for key in ["running", "failed", "queued"] {
                    w.enqueue_task(
                        key.into(),
                        key.into(),
                        move |progress| {
                            progress.stage("检查")?;
                            anyhow::bail!("测试错误")
                        },
                        move |_, result: anyhow::Result<()>, _| {
                            assert_eq!(key, "failed", "cancelled result was published");
                            result
                        },
                        |_, _, _| {},
                        cx,
                    );
                }
                w.background_tasks.cancel(0);
                w.background_tasks.cancel(2);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |w, _, _| {
                let queue = &w.background_tasks;
                assert!(matches!(queue.jobs[&0].state, State::Cancelled));
                assert!(
                    matches!(queue.jobs[&1].state, State::Failed(ref e) if e.contains("测试错误"))
                );
                assert!(matches!(queue.jobs[&2].state, State::Cancelled));
                assert!(queue.active_keys.is_empty());
                assert_eq!(queue.history.len(), 3);
            })
            .unwrap();
    }

    #[gpui::test]
    fn background_queue_bounds_deduplicates_cancels_and_isolates_vaults(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let window = cx.add_window(Workspace::new);
        let applied = Rc::new(std::cell::RefCell::new(Vec::new()));
        window
            .update(cx, |w, _, cx| {
                let add = |w: &mut Workspace, key: &str, cx: &mut Context<Workspace>| {
                    let applied = applied.clone();
                    let label = key.to_owned();
                    w.enqueue_task(
                        label.clone(),
                        label.clone(),
                        |_| Ok(()),
                        move |_, result, _| {
                            result?;
                            applied.borrow_mut().push(label);
                            Ok(())
                        },
                        |_, _, _| {},
                        cx,
                    );
                };
                for key in ["a", "b", "c", "c", "d"] {
                    add(w, key, cx);
                }
                assert_eq!(w.background_tasks.running, 2);
                assert_eq!(w.background_tasks.jobs.len(), 4);
                w.background_tasks.cancel(0);
                w.background_tasks.cancel(2);
                assert!(matches!(
                    w.background_tasks.jobs[&0].state,
                    State::Cancelling
                ));
                assert!(matches!(
                    w.background_tasks.jobs[&2].state,
                    State::Cancelled
                ));
                w.background_tasks.reset();
                add(w, "new", cx);
                assert!(matches!(w.background_tasks.jobs[&4].state, State::Queued));
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(applied.borrow().as_slice(), ["new"]);
        window
            .update(cx, |w, _, _| {
                assert_eq!(w.background_tasks.running, 0);
                assert!(matches!(w.background_tasks.jobs[&4].state, State::Finished));
            })
            .unwrap();
    }
}
