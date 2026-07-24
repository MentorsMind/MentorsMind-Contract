# Forum Contract

A comprehensive on-chain forum contract for the MentorMinds platform, enabling decentralized discussions, Q&A, and knowledge sharing with token-based incentives and reputation systems.

## 🚀 Features

### Core Functionality
- **Post Creation**: Create questions, discussions, announcements, and resource posts
- **Reply System**: Threaded replies with best answer marking
- **Voting**: Weighted voting based on user reputation
- **Moderation**: Community-driven content moderation
- **Categories**: Organized content with customizable categories

### Token Integration
- **Staking**: Minimum MNT token stake required for posting
- **Incentives**: Reputation rewards for quality contributions
- **Best Answer Bonus**: 50 reputation points for accepted answers
- **Moderation Refunds**: Partial stake refunds for moderated content

### Reputation System
- **Dynamic Weight**: Vote weight based on user reputation
- **Multiple Metrics**: Tracks posts, replies, votes received
- **Thresholds**: Reputation requirements for special post types
- **Moderation Access**: High reputation users can moderate content

## 📋 Data Structures

### Post
```rust
pub struct Post {
    pub id: u32,
    pub author: Address,
    pub title: Bytes,
    pub content_hash: BytesN<32>,
    pub post_type: PostType,
    pub category_id: u32,
    pub status: PostStatus,
    pub created_at: u64,
    pub updated_at: u64,
    pub stake_amount: i128,
    pub upvotes: i128,
    pub downvotes: i128,
    pub reply_count: u32,
    pub moderation_count: u32,
}
```

### Reply
```rust
pub struct Reply {
    pub id: u32,
    pub post_id: u32,
    pub author: Address,
    pub content_hash: BytesN<32>,
    pub created_at: u64,
    pub updated_at: u64,
    pub stake_amount: i128,
    pub upvotes: i128,
    pub downvotes: i128,
    pub moderation_count: u32,
    pub is_best_answer: bool,
}
```

### User Reputation
```rust
pub struct UserReputation {
    pub address: Address,
    pub reputation: i128,
    pub posts_created: u32,
    pub replies_created: u32,
    pub upvotes_received: i128,
    pub downvotes_received: i128,
    pub moderation_actions: u32,
}
```

## 🔧 Functions

### Initialization
```rust
initialize(
    admin: Address,
    mnt_token: Address,
    reputation_threshold: Option<i128>,
    min_post_stake: Option<i128>,
    moderation_threshold: Option<u32>,
)
```

### Content Creation
```rust
create_post(
    author: Address,
    title: Bytes,
    content_hash: BytesN<32>,
    post_type: PostType,
    category_id: u32,
    stake_amount: i128,
) -> u32

create_reply(
    author: Address,
    post_id: u32,
    content_hash: BytesN<32>,
    stake_amount: i128,
) -> u32
```

### Interaction
```rust
vote_post(voter: Address, post_id: u32, upvote: bool)
vote_reply(voter: Address, reply_id: u32, upvote: bool)
mark_best_answer(post_author: Address, reply_id: u32)
```

### Moderation
```rust
moderate_content(
    moderator: Address,
    target_id: u32,
    is_reply: bool,
    action: ModerationAction,
    reason: Bytes,
)
```

### Category Management
```rust
create_category(
    admin: Address,
    name: Bytes,
    description: Bytes,
) -> u32
```

### View Functions
```rust
get_post(id: u32) -> Post
get_reply(id: u32) -> Reply
get_category(id: u32) -> Category
get_user_reputation(address: Address) -> UserReputation
get_posts_by_category(category_id: u32) -> Vec<u32>
get_replies_by_post(post_id: u32) -> Vec<u32>
```

## 🎯 Post Types

### Question
- Standard Q&A posts
- No special requirements
- Can mark best answers

### Discussion
- General discussion topics
- No special requirements
- Community-driven conversations

### Announcement
- Official announcements
- Requires 10x reputation threshold
- High visibility posts

### Resource
- Educational resources and guides
- Requires 5x reputation threshold
- Curated content

## 🔐 Access Control

### Reputation Requirements
- **Default**: 0 reputation
- **Announcements**: 10x threshold
- **Resources**: 5x threshold
- **Moderation**: 3 reputation points

### Content Status
- **Active**: Normal visible content
- **Hidden**: Temporarily hidden content
- **Locked**: Read-only content
- **Deleted**: Removed content (with partial refund)

## 💰 Token Economics

### Staking Requirements
- **Default**: 0.001 MNT per post
- **Configurable**: Set by admin during initialization
- **Refunds**: 50% refund on deleted content

### Reputation Rewards
- **Upvote Received**: +vote weight reputation
- **Downvote Received**: -vote weight reputation
- **Best Answer**: +50 reputation
- **Vote Weight**: Based on user's current reputation

## 📊 Default Categories

1. **General Discussion** - General topics and discussions
2. **Technical Support** - Get help with technical issues
3. **Feature Requests** - Suggest new features and improvements
4. **Bug Reports** - Report bugs and issues
5. **Announcements** - Official announcements and news

## 🚀 Deployment

### Build Contract
```bash
cd forum
cargo build --target wasm32-unknown-unknown --release
```

### Optimize WASM
```bash
soroban contract optimize --wasm target/wasm32-unknown-unknown/release/forum.wasm
```

### Deploy to Testnet
```bash
soroban contract deploy \
  --wasm target/wasm32-unknown-unknown/release/forum.wasm \
  --source default \
  --network testnet
```

### Initialize Contract
```bash
soroban contract invoke \
  --id $FORUM_CONTRACT_ID \
  --source default \
  --network testnet \
  -- initialize \
  --admin <admin-address> \
  --mnt-token <token-address> \
  --reputation_threshold 10 \
  --min_post_stake 1000000 \
  --moderation_threshold 3
```

## 🧪 Testing

### Run Unit Tests
```bash
cd forum
cargo test
```

### Test Coverage
- Contract initialization
- Post and reply creation
- Voting system
- Reputation tracking
- Best answer marking
- Content moderation
- Category management

## 🔍 Events

### Content Events
- `post_created` - New post created
- `reply_created` - New reply created
- `post_voted` - Post received vote
- `reply_voted` - Reply received vote
- `best_answer` - Reply marked as best answer

### Moderation Events
- `content_moderated` - Content moderated
- `category_created` - New category created

## 🔗 Integration

### With MNT Token
- Uses MNT for staking and incentives
- Integrates with existing token contracts
- Supports token transfers and balance checks

### With Governance
- Complements governance proposals
- Reputation system aligns with voting power
- Community-driven decision making

### With Escrow
- Forum discussions can reference escrow sessions
- Reputation from forum can influence escrow terms
- Dispute resolution integration possibilities

## 📈 Gas Optimization

- Efficient storage patterns
- Minimal data duplication
- Batch operations for replies
- Optimized vote counting
- Lazy loading for content lists

## 🔒 Security Considerations

- **Access Control**: Proper authorization checks
- **Input Validation**: Validate all inputs and hashes
- **Reentrancy Protection**: Guard against reentrancy attacks
- **Integer Safety**: Checked arithmetic operations
- **Token Safety**: Secure token transfer handling

## 🛠️ Future Enhancements

- **Content Editing**: Allow post editing with version history
- **Tagging System**: Multi-tag support for better categorization
- **Search Functionality**: On-chain search capabilities
- **Trending Algorithm**: Trending content calculation
- **User Profiles**: Extended profile customization
- **Notification System**: Event-based notifications

## 📄 License

MIT License - see LICENSE file for details

---

**Built with Rust and Soroban for the Stellar blockchain**
