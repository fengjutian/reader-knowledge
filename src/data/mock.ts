import type { Book, Note } from "../types/domain";

export const books: Book[] = [
  { id: "1", title: "置身事内", author: "兰小欢", cover: "#b95b45", highlightCount: 38, thoughtCount: 12, progress: 82, updatedAt: "今天" },
  { id: "2", title: "原则", author: "瑞·达利欧", cover: "#244a77", highlightCount: 61, thoughtCount: 18, progress: 100, updatedAt: "昨天" },
  { id: "3", title: "思考，快与慢", author: "丹尼尔·卡尼曼", cover: "#d9a62e", highlightCount: 42, thoughtCount: 9, progress: 63, updatedAt: "9月24日" },
  { id: "4", title: "创新者的窘境", author: "克莱顿·克里斯坦森", cover: "#35675b", highlightCount: 27, thoughtCount: 7, progress: 47, updatedAt: "9月20日" },
];

export const notes: Note[] = [
  { id: "h1", type: "highlight", bookId: "1", bookTitle: "置身事内", chapter: "第五章 城市化与不平衡", content: "地方政府推动经济发展的模式，既塑造了增长，也塑造了结构性约束。", createdAt: "2026-09-27" },
  { id: "t1", type: "thought", bookId: "1", bookTitle: "置身事内", chapter: "第五章 城市化与不平衡", content: "理解组织不能只看制度文本，还要看真实的激励和资源流向。", createdAt: "2026-09-27" },
  { id: "h2", type: "highlight", bookId: "2", bookTitle: "原则", chapter: "第三章 极度开放", content: "可信度加权的决策，比简单多数决策更接近有效判断。", createdAt: "2026-09-25" },
  { id: "t2", type: "thought", bookId: "2", bookTitle: "原则", chapter: "第五章 组织", content: "组织效率和个人能力并不完全一致，系统会放大某些行为，也会压抑另一些行为。", createdAt: "2026-09-24" },
  { id: "h3", type: "highlight", bookId: "3", bookTitle: "思考，快与慢", chapter: "第十九章 确定性错觉", content: "我们对自己判断的信心，并不是判断正确性的可靠指标。", createdAt: "2026-09-22" },
];

