const toggle = document.querySelector(".theme-toggle");
toggle?.addEventListener("click", () => {
  const next = document.documentElement.dataset.theme === "dark" ? "light" : "dark";
  document.documentElement.dataset.theme = next;
  localStorage.setItem("wereader-site-theme", next);
});

const nav = document.querySelector("[data-nav]");
addEventListener(
  "scroll",
  () => nav?.classList.toggle("scrolled", scrollY > 20),
  { passive: true },
);

const observer = new IntersectionObserver(
  (entries) =>
    entries.forEach(
      (entry) => entry.isIntersecting && entry.target.classList.add("visible"),
    ),
  { threshold: 0.12 },
);
document
  .querySelectorAll("section, .reveal")
  .forEach((element) => observer.observe(element));
