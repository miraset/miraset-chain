use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use miraset_core::Address;
use miraset_wallet::Wallet;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph, Tabs},
    Frame, Terminal,
};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ChatMessage {
    from: String,
    message: String,
    timestamp: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct BlockInfo {
    height: u64,
    timestamp: String,
}

enum Tab {
    Wallet,
    Chat,
    Chain,
}

struct App {
    tab: Tab,
    wallet: Wallet,
    selected_account: Option<String>,
    accounts: Vec<(String, Address)>,
    balances: std::collections::HashMap<Address, u64>,
    chat_messages: Vec<ChatMessage>,
    chat_input: String,
    latest_block: Option<BlockInfo>,
    rpc_url: String,
}

impl App {
    fn new(rpc_url: String) -> Result<Self> {
        let wallet_path = get_wallet_path();
        let wallet = Wallet::new(wallet_path)?;
        let accounts = wallet.list_accounts();
        let selected_account = accounts.first().map(|(name, _)| name.clone());

        Ok(Self {
            tab: Tab::Wallet,
            wallet,
            selected_account,
            accounts,
            balances: std::collections::HashMap::new(),
            chat_messages: Vec::new(),
            chat_input: String::new(),
            latest_block: None,
            rpc_url,
        })
    }

    async fn refresh_data(&mut self) -> Result<()> {
        // Refresh accounts
        self.accounts = self.wallet.list_accounts();

        // Refresh balances
        for (_, addr) in &self.accounts {
            if let Ok(balance) = get_balance(&self.rpc_url, addr).await {
                self.balances.insert(*addr, balance);
            }
        }

        // Refresh chat
        if let Ok(messages) = get_chat_messages(&self.rpc_url, 50).await {
            self.chat_messages = messages;
        }

        // Refresh chain info
        if let Ok(block) = get_latest_block(&self.rpc_url).await {
            self.latest_block = Some(block);
        }

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let rpc_url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "http://127.0.0.1:9944".to_string());

    let mut app = App::new(rpc_url)?;

    // Initial data fetch
    let _ = app.refresh_data().await;

    let res = run_app(&mut terminal, &mut app);

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("Error: {:?}", err);
    }

    Ok(())
}

fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> Result<()> {
    let mut last_refresh = std::time::Instant::now();
    let handle = tokio::runtime::Handle::current();

    loop {
        terminal.draw(|f| ui(f, app)).map_err(|e| anyhow::anyhow!("{}", e))?;

        // Auto-refresh every 3 seconds
        if last_refresh.elapsed() > Duration::from_secs(3) {
            let _ = handle.block_on(app.refresh_data());
            last_refresh = std::time::Instant::now();
        }

        // Handle input
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') => return Ok(()),
                        KeyCode::Char('1') => app.tab = Tab::Wallet,
                        KeyCode::Char('2') => app.tab = Tab::Chat,
                        KeyCode::Char('3') => app.tab = Tab::Chain,
                        KeyCode::Char('r') => {
                            let _ = handle.block_on(app.refresh_data());
                        }
                        KeyCode::Char(c) => {
                            if matches!(app.tab, Tab::Chat) {
                                app.chat_input.push(c);
                            }
                        }
                        KeyCode::Backspace => {
                            if matches!(app.tab, Tab::Chat) {
                                app.chat_input.pop();
                            }
                        }
                        KeyCode::Enter => {
                            if matches!(app.tab, Tab::Chat) && !app.chat_input.is_empty() {
                                if let Some(account) = &app.selected_account {
                                    let _ = handle.block_on(send_chat_message(
                                        &app.rpc_url,
                                        &app.wallet,
                                        account,
                                        &app.chat_input,
                                    ));
                                    app.chat_input.clear();
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

fn ui(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0), Constraint::Length(3)])
        .split(f.area());

    // Header tabs
    let titles = vec!["[1] Wallet", "[2] Chat", "[3] Chain", "[Q]uit"];
    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title("Miraset TUI"))
        .select(match app.tab {
            Tab::Wallet => 0,
            Tab::Chat => 1,
            Tab::Chain => 2,
        })
        .style(Style::default().fg(Color::White))
        .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
    f.render_widget(tabs, chunks[0]);

    // Content
    match app.tab {
        Tab::Wallet => render_wallet(f, app, chunks[1]),
        Tab::Chat => render_chat(f, app, chunks[1]),
        Tab::Chain => render_chain(f, app, chunks[1]),
    }

    // Footer
    let footer = Paragraph::new("Press [R] to refresh • Arrow keys to navigate")
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[2]);
}

fn render_wallet(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Accounts list
    let items: Vec<ListItem> = app
        .accounts
        .iter()
        .map(|(name, addr)| {
            let balance = app.balances.get(addr).copied().unwrap_or(0);
            let content = format!(
                "{}: {} (balance: {})",
                name,
                &addr.to_hex()[..12],
                balance
            );
            ListItem::new(content)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Accounts"))
        .style(Style::default().fg(Color::White));
    f.render_widget(list, chunks[0]);

    // Info
    let info = if app.accounts.is_empty() {
        "No accounts. Use CLI: miraset wallet new <name>".to_string()
    } else {
        format!("Total accounts: {}", app.accounts.len())
    };
    let info_widget = Paragraph::new(info)
        .block(Block::default().borders(Borders::ALL).title("Info"));
    f.render_widget(info_widget, chunks[1]);
}

fn render_chat(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(area);

    // Messages
    let items: Vec<ListItem> = app
        .chat_messages
        .iter()
        .map(|msg| {
            let content = format!("[{}] {}: {}", msg.timestamp, &msg.from[..8], msg.message);
            ListItem::new(content)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Chat Messages"))
        .style(Style::default().fg(Color::White));
    f.render_widget(list, chunks[0]);

    // Input
    let input = Paragraph::new(app.chat_input.as_str())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Type message (Enter to send)"),
        );
    f.render_widget(input, chunks[1]);
}

fn render_chain(f: &mut Frame, app: &App, area: Rect) {
    let info = if let Some(block) = &app.latest_block {
        format!(
            "Latest Block Height: {}\nTimestamp: {}",
            block.height, block.timestamp
        )
    } else {
        "Loading chain info...".to_string()
    };

    let widget = Paragraph::new(info)
        .block(Block::default().borders(Borders::ALL).title("Chain Info"));
    f.render_widget(widget, area);
}

fn get_wallet_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".miraset").join("wallet.json")
}

async fn get_balance(rpc: &str, addr: &Address) -> Result<u64> {
    let url = format!("{}/balance/{}", rpc, addr.to_hex());
    let resp = reqwest::get(&url).await?;
    Ok(resp.json().await?)
}

async fn get_chat_messages(rpc: &str, limit: usize) -> Result<Vec<ChatMessage>> {
    let url = format!("{}/chat/messages?limit={}", rpc, limit);
    let resp = reqwest::get(&url).await?;
    Ok(resp.json().await?)
}

async fn get_latest_block(rpc: &str) -> Result<BlockInfo> {
    let url = format!("{}/block/latest", rpc);
    let resp = reqwest::get(&url).await?;
    let block: serde_json::Value = resp.json().await?;
    Ok(BlockInfo {
        height: block["height"].as_u64().unwrap_or(0),
        timestamp: block["timestamp"].as_str().unwrap_or("").to_string(),
    })
}

async fn send_chat_message(
    rpc: &str,
    wallet: &Wallet,
    account: &str,
    message: &str,
) -> Result<()> {
    use miraset_core::Transaction;

    let kp = wallet.get_keypair(account)?;
    let nonce = get_nonce(rpc, &kp.address()).await?;

    let mut tx = Transaction::ChatSend {
        from: kp.address(),
        message: message.to_string(),
        nonce,
        signature: [0; 64],
    };

    let msg_bytes = bincode::serialize(&tx)?;
    let sig = kp.sign(&msg_bytes);
    if let Transaction::ChatSend { signature, .. } = &mut tx {
        *signature = sig;
    }

    submit_tx(rpc, &tx).await?;
    Ok(())
}

async fn get_nonce(rpc: &str, addr: &Address) -> Result<u64> {
    let url = format!("{}/nonce/{}", rpc, addr.to_hex());
    let resp = reqwest::get(&url).await?;
    Ok(resp.json().await?)
}

async fn submit_tx(rpc: &str, tx: &miraset_core::Transaction) -> Result<()> {
    let url = format!("{}/tx/submit", rpc);
    let client = reqwest::Client::new();
    let resp = client.post(&url).json(tx).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("TX failed: {}", resp.text().await?);
    }
    Ok(())
}
